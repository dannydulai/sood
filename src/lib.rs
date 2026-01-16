//! # Sood - Service-Oriented Object Discovery
//!
//! A Rust implementation of the Sood discovery protocol used by Roon for discovering
//! the Core as well as audio endpoints on the local network.
//!
//! ## Overview
//!
//! Sood is a UDP-based service discovery protocol that uses multicast and broadcast
//! to discover devices on the local network. This crate provides a simple async API
//! for sending discovery queries and receiving responses.
//!
//! ## Protocol Details
//!
//! - **Multicast Address**: 239.255.90.90
//! - **Port**: 9003
//! - **Protocol Version**: 2
//!
//! The protocol sends queries to both multicast and broadcast addresses on all
//! network interfaces, and listens for responses on the same addresses.
//!
//! ## Example
//!
//! ```no_run
//! use sood::{service_ids, Sood};
//!
//! #[tokio::main]
//! async fn main() -> std::io::Result<()> {
//!     // Create and start the discovery client
//!     let mut sood = Sood::new()?;
//!     sood.start().await?;
//!
//!     // Get a receiver for discovered devices
//!     let mut devices = sood.discovered().await;
//!
//!     // Send a discovery query using a well-known service ID
//!     sood.discover_service(service_ids::ROON_API).await?;
//!
//!     // Listen for devices (automatically filtered and deduplicated)
//!     while let Ok(device) = devices.recv().await {
//!         println!("Discovered: {} at {}", device.unique_id, device.from);
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Features
//!
//! - Async/await API using Tokio
//! - Automatic network interface monitoring
//! - Multicast and broadcast discovery
//! - Handles network changes gracefully
//! - Transaction ID auto-generation

use std::collections::HashMap;
use std::io;
use tokio::sync::broadcast;

mod discovery;
pub mod network;
pub mod protocol;

pub use protocol::{Message, MessageType};

/// Well-known Roon service IDs
pub mod service_ids {
    /// Roon API service ID - used by Roon extensions and control applications
    pub const ROON_API: &str = "00720724-5143-4a9b-abac-0e50cba674bb";

    /// Roon Server service ID - the main Roon Core server
    pub const ROON_SERVER: &str = "d52b2cb7-02c5-48fc-981b-a10f0aadd93b";

    /// Roon OS service ID - Roon OS/Nucleus devices
    pub const ROON_OS: &str = "ecb37afa-665f-4693-9fba-54823b6f1ff1";

    /// RAAT Endpoint service ID - Roon OS/Nucleus devices
    pub const RAAT_ENDPOINT: &str = "5e2042ad-9bc5-4508-be92-ff68f19bdc93";
}

/// A receiver for query responses matching a specific transaction ID
///
/// This receiver will yield response messages that match the transaction ID
/// of the query that was sent. The receiver automatically closes after 5 seconds.
pub struct QueryResponses {
    receiver: broadcast::Receiver<Message>,
}

impl QueryResponses {
    fn new(receiver: broadcast::Receiver<Message>) -> Self {
        Self { receiver }
    }

    /// Receive the next response message
    ///
    /// Returns `None` when the stream has closed (after 5 seconds) or an error occurs.
    pub async fn recv(&mut self) -> Option<Message> {
        match self.receiver.recv().await {
            Ok(msg) => Some(msg),
            Err(_) => None,
        }
    }
}

/// A receiver for discovered devices that includes past discoveries
///
/// This receiver will first yield all devices discovered before the receiver
/// was created, then switch to receiving new devices as they are discovered.
pub struct DiscoveredReceiver {
    past_devices: Vec<DiscoveredDevice>,
    past_index: usize,
    live_receiver: broadcast::Receiver<DiscoveredDevice>,
}

impl DiscoveredReceiver {
    fn new(
        past_devices: Vec<DiscoveredDevice>,
        live_receiver: broadcast::Receiver<DiscoveredDevice>,
    ) -> Self {
        Self {
            past_devices,
            past_index: 0,
            live_receiver,
        }
    }

    /// Receive the next discovered device
    ///
    /// This will first return all devices discovered before this receiver was created,
    /// then receive new devices as they are discovered.
    pub async fn recv(&mut self) -> Result<DiscoveredDevice, broadcast::error::RecvError> {
        // First, drain past devices
        if self.past_index < self.past_devices.len() {
            let device = self.past_devices[self.past_index].clone();
            self.past_index += 1;
            return Ok(device);
        }

        // Then switch to live receiver
        self.live_receiver.recv().await
    }
}

/// A discovered device with its properties
///
/// This type represents a device that has been discovered on the network and has both
/// a service ID and unique ID. Devices are automatically deduplicated by their
/// (service_id, unique_id) combination.
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    /// Device address
    pub from: std::net::SocketAddr,
    /// Service ID
    pub service_id: String,
    /// Unique device ID
    pub unique_id: String,
    /// All device properties
    pub properties: HashMap<String, Option<String>>,
}

/// SOOD discovery client
///
/// This is the main entry point for using the Sood protocol. Create an instance,
/// start it, and then send queries and receive messages.
///
/// # Example
///
/// ```no_run
/// use sood::{service_ids, Sood};
///
/// #[tokio::main]
/// async fn main() -> std::io::Result<()> {
///     let mut sood = Sood::new()?;
///     sood.start().await?;
///
///     let mut devices = sood.discovered().await;
///     sood.discover_service(service_ids::ROON_API).await?;
///
///     while let Ok(device) = devices.recv().await {
///         println!("Found: {} at {}", device.unique_id, device.from);
///     }
///
///     Ok(())
/// }
/// ```
pub struct Sood {
    discovery: discovery::SoodDiscovery,
}

impl Sood {
    /// Create a new SOOD discovery client
    ///
    /// This initializes the client but does not start listening or monitoring
    /// network interfaces. Call [`start()`](Self::start) to begin discovery.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            discovery: discovery::SoodDiscovery::new(),
        })
    }

    /// Start the discovery client
    ///
    /// This begins listening on all network interfaces and starts monitoring
    /// for network changes.
    ///
    /// # Errors
    ///
    /// Returns an error if socket setup fails (e.g., permission issues).
    pub async fn start(&mut self) -> io::Result<()> {
        self.discovery.start().await
    }

    /// Send a discovery query and receive responses
    ///
    /// Sends a query message and returns a stream of responses with matching transaction IDs.
    /// The stream automatically closes after 5 seconds.
    ///
    /// # Arguments
    ///
    /// * `properties` - Key-value pairs to include in the query. Use `None` for null values.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use sood::Sood;
    /// # use std::collections::HashMap;
    /// # async fn example(sood: &Sood) {
    /// let mut props = HashMap::new();
    /// props.insert("service".to_string(), Some("roon".to_string()));
    /// props.insert("version".to_string(), Some("1.0".to_string()));
    /// let mut responses = sood.query(props).await.unwrap();
    ///
    /// while let Some(response) = responses.recv().await {
    ///     println!("Got response: {:?}", response);
    /// }
    /// # }
    /// ```
    pub async fn query(&self, properties: HashMap<String, Option<String>>) -> io::Result<QueryResponses> {
        let receiver = self.discovery.query(properties).await?;
        Ok(QueryResponses::new(receiver))
    }

    /// Discover devices by service ID
    ///
    /// This is a convenience method for the common case of discovering devices
    /// by their service ID. Devices will be available through the [`discovered()`](Self::discovered)
    /// receiver. For more complex queries with multiple properties, use [`query()`](Self::query).
    ///
    /// # Arguments
    ///
    /// * `service_id` - The service UUID to query for
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use sood::Sood;
    /// # async fn example(sood: &Sood) {
    /// sood.discover_service("00720724-5143-4a9b-abac-0e50cba674bb").await.unwrap();
    /// # }
    /// ```
    pub async fn discover_service(&self, service_id: impl Into<String>) -> io::Result<()> {
        let mut properties = HashMap::new();
        properties.insert("query_service_id".to_string(), Some(service_id.into()));
        let _responses = self.query(properties).await?;
        // Responses are automatically handled by the discovered() receiver
        // We don't need to consume them here
        Ok(())
    }

    /// Get a receiver for all messages (responses only, no query reflections)
    ///
    /// Returns a broadcast receiver that will receive all response messages.
    /// Query reflections are filtered out. Multiple receivers can be created,
    /// and each will receive all messages.
    ///
    /// For a simpler API that automatically filters and deduplicates devices,
    /// use [`discovered()`](Self::discovered).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use sood::Sood;
    /// # async fn example(sood: &Sood) {
    /// let mut rx1 = sood.messages();
    /// let mut rx2 = sood.messages(); // Independent receiver
    ///
    /// tokio::spawn(async move {
    ///     while let Ok(msg) = rx1.recv().await {
    ///         println!("Receiver 1: {:?}", msg);
    ///     }
    /// });
    ///
    /// while let Ok(msg) = rx2.recv().await {
    ///     println!("Receiver 2: {:?}", msg);
    /// }
    /// # }
    /// ```
    pub fn messages(&self) -> broadcast::Receiver<Message> {
        self.discovery.subscribe()
    }

    /// Get a receiver for discovered devices (filtered and deduplicated)
    ///
    /// This receiver only emits devices with both `service_id` and `unique_id`,
    /// and automatically filters out duplicates. Each device will only be
    /// emitted once per session.
    ///
    /// **Important:** This receiver includes all devices discovered before it was
    /// created, so there's no race condition. You can call this before or after
    /// `start()` and you'll receive all devices.
    ///
    /// For raw message access, use [`messages()`](Self::messages).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use sood::Sood;
    /// # async fn example(sood: &Sood) {
    /// let mut devices = sood.discovered().await;
    ///
    /// while let Ok(device) = devices.recv().await {
    ///     println!("Found: {} at {}", device.unique_id, device.from);
    /// }
    /// # }
    /// ```
    pub async fn discovered(&self) -> DiscoveredReceiver {
        let (past_devices, live_receiver) = self.discovery.subscribe_discovered_with_history().await;
        DiscoveredReceiver::new(past_devices, live_receiver)
    }

    /// Get all discovered devices
    ///
    /// Returns a snapshot of all devices that have been discovered so far.
    /// This is useful for getting the current state without listening to
    /// the stream.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use sood::Sood;
    /// # async fn example(sood: &Sood) {
    /// let devices = sood.get_discovered_devices().await;
    /// println!("Found {} devices", devices.len());
    /// for device in devices {
    ///     println!("  - {} at {}", device.unique_id, device.from);
    /// }
    /// # }
    /// ```
    pub async fn get_discovered_devices(&self) -> Vec<DiscoveredDevice> {
        self.discovery.get_discovered_devices().await
    }

    /// Stop the discovery client
    ///
    /// Stops all listening sockets and the network monitoring task.
    /// This consumes the `Sood` instance.
    pub async fn stop(self) {
        self.discovery.stop().await;
    }
}

impl Default for Sood {
    fn default() -> Self {
        Self::new().expect("Failed to create Sood instance")
    }
}

/// A builder for creating a Sood instance with custom configuration
///
/// Currently this is a simple wrapper, but can be extended in the future
/// to support custom ports, multicast addresses, etc.
pub struct SoodBuilder {
    _private: (),
}

impl SoodBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Build the Sood instance
    pub fn build(self) -> io::Result<Sood> {
        Sood::new()
    }
}

impl Default for SoodBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_sood() {
        let sood = Sood::new();
        assert!(sood.is_ok());
    }

    #[tokio::test]
    async fn test_builder() {
        let sood = SoodBuilder::new().build();
        assert!(sood.is_ok());
    }
}
