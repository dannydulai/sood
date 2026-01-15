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
//! use sood::Sood;
//! use std::collections::HashMap;
//!
//! #[tokio::main]
//! async fn main() -> std::io::Result<()> {
//!     // Create and start the discovery client
//!     let mut sood = Sood::new()?;
//!     let mut messages = sood.messages();
//!
//!     sood.start().await?;
//!
//!     // Send a discovery query
//!     let mut properties = HashMap::new();
//!     properties.insert("service".to_string(), Some("roon".to_string()));
//!     sood.query(properties).await?;
//!
//!     // Listen for responses
//!     while let Ok(msg) = messages.recv().await {
//!         println!("Discovered device at {}", msg.from);
//!         println!("  Properties: {:?}", msg.properties);
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

/// SOOD discovery client
///
/// This is the main entry point for using the Sood protocol. Create an instance,
/// start it, and then send queries and receive messages.
///
/// # Example
///
/// ```no_run
/// use sood::Sood;
/// use std::collections::HashMap;
///
/// #[tokio::main]
/// async fn main() -> std::io::Result<()> {
///     let mut sood = Sood::new()?;
///     let mut messages = sood.messages();
///
///     sood.start().await?;
///
///     let mut props = HashMap::new();
///     props.insert("query_service_id".to_string(), Some("com.roon.app".to_string()));
///     sood.query(props).await?;
///
///     while let Ok(msg) = messages.recv().await {
///         println!("Found: {:?}", msg);
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

    /// Send a discovery query
    ///
    /// Sends a query message
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
    /// sood.query(props).await.unwrap();
    /// # }
    /// ```
    pub async fn query(&self, properties: HashMap<String, Option<String>>) -> io::Result<()> {
        self.discovery.query(properties).await
    }

    /// Discover devices by service ID
    ///
    /// This is a convenience method for the common case of discovering devices
    /// by their service ID. For more complex queries with multiple properties,
    /// use [`query()`](Self::query).
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
        self.query(properties).await
    }

    /// Get a receiver for discovered messages
    ///
    /// Returns a broadcast receiver that will receive all discovered messages.
    /// Multiple receivers can be created, and each will receive all messages.
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
