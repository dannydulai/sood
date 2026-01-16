//! Core SOOD discovery implementation.
//!
//! This module provides the main [`SoodDiscovery`] struct that manages network
//! sockets, sends queries, and receives messages.

use crate::network::{get_network_interfaces, NetworkInterface};
use crate::protocol::{self, Message, MessageType};
use crate::DiscoveredDevice;
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

const SOOD_PORT: u16 = 9003;
const SOOD_MULTICAST_IP: Ipv4Addr = Ipv4Addr::new(239, 255, 90, 90);
const INTERFACE_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const MESSAGE_BUFFER_SIZE: usize = 65535;
const BROADCAST_CHANNEL_CAPACITY: usize = 100;

/// Internal state for a network interface
struct InterfaceState {
    info: NetworkInterface,
    recv_task: Option<JoinHandle<()>>,
    send_socket: Option<Arc<UdpSocket>>,
}

/// Internal discovery engine state
struct DiscoveryState {
    interfaces: HashMap<String, InterfaceState>,
    unicast_socket: Option<Arc<UdpSocket>>,
    monitor_task: Option<JoinHandle<()>>,
    /// Track discovered devices by (service_id, unique_id) to prevent duplicates
    discovered_devices: Arc<RwLock<HashMap<(String, String), DiscoveredDevice>>>,
    /// Track active queries by transaction ID (UUID)
    active_queries: Arc<RwLock<HashMap<Uuid, broadcast::Sender<Message>>>>,
}

/// SOOD discovery engine
///
/// This is the internal implementation that manages sockets and network monitoring.
/// Users should interact with the public `Sood` API in lib.rs instead.
pub struct SoodDiscovery {
    state: Arc<RwLock<DiscoveryState>>,
    message_tx: broadcast::Sender<Message>,
    _message_rx: broadcast::Receiver<Message>,
    /// Channel for discovered devices (filtered and deduplicated)
    discovered_tx: broadcast::Sender<DiscoveredDevice>,
    _discovered_rx: broadcast::Receiver<DiscoveredDevice>,
}

impl SoodDiscovery {
    /// Create a new SOOD discovery engine
    pub fn new() -> Self {
        let (message_tx, message_rx) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let (discovered_tx, discovered_rx) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);

        let state = DiscoveryState {
            interfaces: HashMap::new(),
            unicast_socket: None,
            monitor_task: None,
            discovered_devices: Arc::new(RwLock::new(HashMap::new())),
            active_queries: Arc::new(RwLock::new(HashMap::new())),
        };

        Self {
            state: Arc::new(RwLock::new(state)),
            message_tx,
            _message_rx: message_rx,
            discovered_tx,
            _discovered_rx: discovered_rx,
        }
    }

    /// Get a receiver for all messages (responses only, no query reflections)
    pub fn subscribe(&self) -> broadcast::Receiver<Message> {
        self.message_tx.subscribe()
    }

    /// Get a receiver for discovered devices with past devices included
    pub async fn subscribe_discovered_with_history(
        &self,
    ) -> (Vec<DiscoveredDevice>, broadcast::Receiver<DiscoveredDevice>) {
        let past_devices = self.get_discovered_devices().await;
        let receiver = self.discovered_tx.subscribe();
        (past_devices, receiver)
    }

    /// Get all discovered devices
    ///
    /// Returns a snapshot of all devices discovered so far.
    pub async fn get_discovered_devices(&self) -> Vec<DiscoveredDevice> {
        let state = self.state.read().await;
        let devices = state.discovered_devices.read().await;
        devices.values().cloned().collect()
    }

    /// Start the discovery engine
    pub async fn start(&mut self) -> io::Result<()> {
        // Initialize sockets for all interfaces
        self.init_sockets().await?;

        // Start the interface monitoring task
        let state = Arc::clone(&self.state);
        let message_tx = self.message_tx.clone();
        let discovered_tx = self.discovered_tx.clone();
        let monitor_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(INTERFACE_CHECK_INTERVAL);
            loop {
                interval.tick().await;
                if let Err(e) = Self::check_interfaces(
                    Arc::clone(&state),
                    message_tx.clone(),
                    discovered_tx.clone(),
                )
                .await
                {
                    tracing::warn!("Interface check failed: {}", e);
                }
            }
        });

        self.state.write().await.monitor_task = Some(monitor_task);

        Ok(())
    }

    /// Send a SOOD query and return a stream of responses
    ///
    /// The returned stream will receive responses with a matching transaction ID.
    /// The stream automatically closes after 5 seconds.
    pub async fn query(
        &self,
        mut properties: HashMap<String, Option<String>>,
    ) -> io::Result<broadcast::Receiver<Message>> {
        let buf = protocol::serialize_query(&mut properties);

        // Get the transaction ID that was assigned (or generated) and parse it as UUID
        let tid_str = properties
            .get("_tid")
            .and_then(|v| v.as_ref())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing _tid"))?;

        let tid = Uuid::parse_str(tid_str).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Invalid UUID for _tid: {}", e),
            )
        })?;

        // Create a channel for responses to this query
        let (query_tx, query_rx) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);

        // Store the channel in active_queries
        {
            let state = self.state.read().await;
            let mut active_queries = state.active_queries.write().await;
            active_queries.insert(tid, query_tx);
        }

        // Spawn a task to remove the query after 5 seconds
        let state_clone = Arc::clone(&self.state);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let state = state_clone.read().await;
            let mut active_queries = state.active_queries.write().await;
            active_queries.remove(&tid);
            tracing::debug!("Query stream for tid {} closed after timeout", tid);
        });

        let state = self.state.read().await;

        // Send via all multicast interfaces
        for iface_state in state.interfaces.values() {
            if let Some(socket) = &iface_state.send_socket {
                // Send to multicast address
                let multicast_addr = SocketAddr::V4(SocketAddrV4::new(SOOD_MULTICAST_IP, SOOD_PORT));
                if let Err(e) = socket.send_to(&buf, multicast_addr).await {
                    tracing::warn!(
                        "Failed to send to multicast on {}: {}",
                        iface_state.info.name,
                        e
                    );
                }

                // Send to broadcast address
                let broadcast_addr = SocketAddr::V4(SocketAddrV4::new(iface_state.info.broadcast, SOOD_PORT));
                if let Err(e) = socket.send_to(&buf, broadcast_addr).await {
                    tracing::warn!(
                        "Failed to send to broadcast on {}: {}",
                        iface_state.info.name,
                        e
                    );
                }
            }
        }

        // Send via unicast socket
        if let Some(socket) = &state.unicast_socket {
            let multicast_addr = SocketAddr::V4(SocketAddrV4::new(SOOD_MULTICAST_IP, SOOD_PORT));
            if let Err(e) = socket.send_to(&buf, multicast_addr).await {
                tracing::warn!("Failed to send via unicast socket: {}", e);
            }
        }

        Ok(query_rx)
    }

    /// Stop the discovery engine
    pub async fn stop(self) {
        let mut state = self.state.write().await;

        // Stop monitoring task
        if let Some(task) = state.monitor_task.take() {
            task.abort();
        }

        // Stop all interface tasks
        for (_, iface_state) in state.interfaces.drain() {
            if let Some(task) = iface_state.recv_task {
                task.abort();
            }
        }
    }

    /// Initialize sockets for all network interfaces
    async fn init_sockets(&self) -> io::Result<()> {
        let interfaces = get_network_interfaces();
        let mut state = self.state.write().await;

        // Create unicast socket if it doesn't exist
        if state.unicast_socket.is_none() {
            let socket = Self::create_unicast_socket()?;
            let socket = Arc::new(socket);

            // Spawn receive task for unicast socket
            let recv_socket = Arc::clone(&socket);
            let message_tx = self.message_tx.clone();
            let discovered_tx = self.discovered_tx.clone();
            let discovered_devices = Arc::clone(&state.discovered_devices);
            let active_queries = Arc::clone(&state.active_queries);
            tokio::spawn(async move {
                Self::receive_loop(
                    recv_socket,
                    message_tx,
                    discovered_tx,
                    discovered_devices,
                    active_queries,
                )
                .await;
            });

            state.unicast_socket = Some(socket);
        }

        // Add or update interfaces
        for iface in interfaces {
            let name = iface.name.clone();

            if !state.interfaces.contains_key(&name) {
                // New interface - create sockets
                match Self::setup_interface(
                    iface,
                    self.message_tx.clone(),
                    self.discovered_tx.clone(),
                    Arc::clone(&state.discovered_devices),
                    Arc::clone(&state.active_queries),
                )
                .await
                {
                    Ok(iface_state) => {
                        state.interfaces.insert(name, iface_state);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to setup interface {}: {}", name, e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Check for interface changes
    async fn check_interfaces(
        state: Arc<RwLock<DiscoveryState>>,
        message_tx: broadcast::Sender<Message>,
        discovered_tx: broadcast::Sender<DiscoveredDevice>,
    ) -> io::Result<()> {
        let current_interfaces = get_network_interfaces();
        let mut state = state.write().await;

        // Build a set of current interface names
        let current_names: std::collections::HashSet<String> =
            current_interfaces.iter().map(|i| i.name.clone()).collect();

        // Remove interfaces that no longer exist
        state.interfaces.retain(|name, iface_state| {
            if !current_names.contains(name) {
                tracing::info!("Interface {} removed", name);
                if let Some(task) = &iface_state.recv_task {
                    task.abort();
                }
                false
            } else {
                true
            }
        });

        // Add new interfaces
        for iface in current_interfaces {
            let name = iface.name.clone();
            if !state.interfaces.contains_key(&name) {
                tracing::info!("New interface detected: {}", name);
                let discovered_devices = Arc::clone(&state.discovered_devices);
                let active_queries = Arc::clone(&state.active_queries);
                match Self::setup_interface(
                    iface,
                    message_tx.clone(),
                    discovered_tx.clone(),
                    discovered_devices,
                    active_queries,
                )
                .await
                {
                    Ok(iface_state) => {
                        state.interfaces.insert(name, iface_state);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to setup new interface: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Setup sockets for a single interface
    async fn setup_interface(
        info: NetworkInterface,
        message_tx: broadcast::Sender<Message>,
        discovered_tx: broadcast::Sender<DiscoveredDevice>,
        discovered_devices: Arc<RwLock<HashMap<(String, String), DiscoveredDevice>>>,
        active_queries: Arc<RwLock<HashMap<Uuid, broadcast::Sender<Message>>>>,
    ) -> io::Result<InterfaceState> {
        // Create multicast receive socket
        let recv_socket = Self::create_multicast_recv_socket(info.ip)?;
        let recv_socket = Arc::new(recv_socket);

        // Spawn receive task
        let recv_task_socket = Arc::clone(&recv_socket);
        let recv_task_tx = message_tx.clone();
        let recv_task_discovered_tx = discovered_tx.clone();
        let recv_task_discovered_devices = Arc::clone(&discovered_devices);
        let recv_task_active_queries = Arc::clone(&active_queries);
        let recv_task = tokio::spawn(async move {
            Self::receive_loop(
                recv_task_socket,
                recv_task_tx,
                recv_task_discovered_tx,
                recv_task_discovered_devices,
                recv_task_active_queries,
            )
            .await;
        });

        // Create send socket
        let send_socket = Self::create_multicast_send_socket(info.ip)?;
        let send_socket = Arc::new(send_socket);

        // IMPORTANT: Also listen for responses on the send socket!
        // Devices respond to the source IP:port of queries, which is this send socket
        let send_socket_rx = Arc::clone(&send_socket);
        let send_socket_tx = message_tx.clone();
        let send_socket_discovered_tx = discovered_tx.clone();
        let send_socket_discovered_devices = Arc::clone(&discovered_devices);
        let send_socket_active_queries = Arc::clone(&active_queries);
        tokio::spawn(async move {
            Self::receive_loop(
                send_socket_rx,
                send_socket_tx,
                send_socket_discovered_tx,
                send_socket_discovered_devices,
                send_socket_active_queries,
            )
            .await;
        });

        Ok(InterfaceState {
            info,
            recv_task: Some(recv_task),
            send_socket: Some(send_socket),
        })
    }

    /// Create a multicast receive socket bound to SOOD_PORT
    fn create_multicast_recv_socket(interface_ip: Ipv4Addr) -> io::Result<UdpSocket> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;

        // Set socket options
        socket.set_reuse_address(true)?;
        #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
        socket.set_reuse_port(true)?;

        // Bind to SOOD_PORT on all interfaces
        let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, SOOD_PORT);
        socket.bind(&socket2::SockAddr::from(addr))?;

        // Join multicast group on this interface
        socket.join_multicast_v4(&SOOD_MULTICAST_IP, &interface_ip)?;

        // Convert to tokio UdpSocket
        socket.set_nonblocking(true)?;
        UdpSocket::from_std(socket.into())
    }

    /// Create a multicast send socket bound to a specific interface
    fn create_multicast_send_socket(interface_ip: Ipv4Addr) -> io::Result<UdpSocket> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;

        // Set socket options
        socket.set_broadcast(true)?;
        socket.set_multicast_ttl_v4(1)?; // Local network only

        // Bind to interface IP with ephemeral port
        let addr = SocketAddrV4::new(interface_ip, 0);
        socket.bind(&socket2::SockAddr::from(addr))?;

        // Convert to tokio UdpSocket
        socket.set_nonblocking(true)?;
        UdpSocket::from_std(socket.into())
    }

    /// Create a unicast socket
    fn create_unicast_socket() -> io::Result<UdpSocket> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;

        // Set socket options
        socket.set_broadcast(true)?;
        socket.set_multicast_ttl_v4(1)?;

        // Bind to any address with ephemeral port
        let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
        socket.bind(&socket2::SockAddr::from(addr))?;

        // Convert to tokio UdpSocket
        socket.set_nonblocking(true)?;
        UdpSocket::from_std(socket.into())
    }

    /// Receive loop for a socket
    async fn receive_loop(
        socket: Arc<UdpSocket>,
        message_tx: broadcast::Sender<Message>,
        discovered_tx: broadcast::Sender<DiscoveredDevice>,
        discovered_devices: Arc<RwLock<HashMap<(String, String), DiscoveredDevice>>>,
        active_queries: Arc<RwLock<HashMap<Uuid, broadcast::Sender<Message>>>>,
    ) {
        let mut buf = vec![0u8; MESSAGE_BUFFER_SIZE];

        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, from)) => {
                    tracing::debug!("Received {} bytes from {}", len, from);
                    tracing::trace!("Raw data: {:02X?}", &buf[..len.min(100)]);

                    if let Some(msg) = protocol::parse_message(&buf[..len], from) {
                        tracing::debug!("Successfully parsed message from {}", from);

                        // Handle Response messages
                        if msg.msg_type == MessageType::Response {
                            // Check if this response matches any active query
                            if let Some(Some(tid_str)) = msg.properties.get("_tid") {
                                // Parse the _tid as a UUID
                                if let Ok(tid) = Uuid::parse_str(tid_str) {
                                    let queries = active_queries.read().await;
                                    if let Some(query_tx) = queries.get(&tid) {
                                        // Send to the specific query stream
                                        let _ = query_tx.send(msg.clone());
                                    }
                                }
                            }

                            // Also send to general messages channel (for backward compatibility)
                            let _ = message_tx.send(msg.clone());
                        }

                        // Check if it's a device announcement (Query or Response with service_id and unique_id)
                        // This handles both unsolicited Query announcements and Response messages
                        if let Some(Some(service_id)) = msg.properties.get("service_id") {
                            // Special case: ROON_OS uses serial_number instead of unique_id
                            let unique_id = if service_id == "ecb37afa-665f-4693-9fba-54823b6f1ff1" {
                                msg.properties.get("serial_number")
                                    .and_then(|v| v.as_ref())
                            } else {
                                msg.properties.get("unique_id")
                                    .and_then(|v| v.as_ref())
                            };

                            if let Some(unique_id) = unique_id {
                                let key = (service_id.clone(), unique_id.clone());

                                // Check if we've seen this device before
                                let mut devices = discovered_devices.write().await;
                                if !devices.contains_key(&key) {
                                    // New device - store and emit it
                                    let device = DiscoveredDevice {
                                        from: msg.from,
                                        service_id: service_id.clone(),
                                        unique_id: unique_id.clone(),
                                        properties: msg.properties.clone(),
                                    };
                                    devices.insert(key, device.clone());
                                    let _ = discovered_tx.send(device);
                                }
                            }
                        }
                    } else {
                        tracing::debug!("Failed to parse message from {}", from);
                    }
                }
                Err(e) => {
                    tracing::debug!("Socket receive error: {}", e);
                    // Don't break - temporary errors are common
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
}

impl Default for SoodDiscovery {
    fn default() -> Self {
        Self::new()
    }
}
