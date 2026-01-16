//! Network interface enumeration and utilities.
//!
//! This module provides functionality to discover network interfaces and calculate
//! broadcast addresses for SOOD protocol communication.

use std::net::Ipv4Addr;

/// Network interface information
#[derive(Debug, Clone)]
pub struct NetworkInterface {
    /// Interface name (e.g., "eth0", "wlan0")
    pub name: String,
    /// IPv4 address assigned to this interface
    pub ip: Ipv4Addr,
    /// Network mask
    pub netmask: Ipv4Addr,
    /// Broadcast address for this network
    pub broadcast: Ipv4Addr,
}

/// Get all IPv4 network interfaces on the system
///
/// This function enumerates all network interfaces and returns those with
/// IPv4 addresses, excluding loopback interfaces.
///
/// # Returns
///
/// A vector of `NetworkInterface` structures, one for each valid IPv4 interface.
pub fn get_network_interfaces() -> Vec<NetworkInterface> {
    let mut interfaces = Vec::new();

    if let Ok(addrs) = if_addrs::get_if_addrs() {
        for iface in addrs {
            // Skip loopback interfaces
            if iface.is_loopback() {
                continue;
            }

            // Only process IPv4 addresses
            if let if_addrs::IfAddr::V4(addr) = iface.addr {
                let ip = addr.ip;
                let netmask = addr.netmask;
                let broadcast = addr.broadcast.unwrap_or_else(|| {
                    // Calculate broadcast if not provided
                    calculate_broadcast(ip, netmask)
                });

                interfaces.push(NetworkInterface {
                    name: iface.name,
                    ip,
                    netmask,
                    broadcast,
                });
            }
        }
    }

    interfaces
}

/// Get all IPv4 loopback interfaces on the system
///
/// This function enumerates all network interfaces and returns only loopback interfaces
/// with IPv4 addresses.
///
/// # Returns
///
/// A vector of `NetworkInterface` structures, one for each loopback interface.
pub fn get_loopback_interfaces() -> Vec<NetworkInterface> {
    let mut interfaces = Vec::new();

    if let Ok(addrs) = if_addrs::get_if_addrs() {
        for iface in addrs {
            // Only include loopback interfaces
            if !iface.is_loopback() {
                continue;
            }

            // Only process IPv4 addresses
            if let if_addrs::IfAddr::V4(addr) = iface.addr {
                let ip = addr.ip;
                let netmask = addr.netmask;
                let broadcast = addr.broadcast.unwrap_or_else(|| {
                    // Calculate broadcast if not provided
                    calculate_broadcast(ip, netmask)
                });

                interfaces.push(NetworkInterface {
                    name: iface.name,
                    ip,
                    netmask,
                    broadcast,
                });
            }
        }
    }

    interfaces
}

/// Calculate the broadcast address for a given IP and netmask
///
/// The broadcast address is calculated by setting all host bits to 1.
///
/// # Arguments
///
/// * `ip` - The IPv4 address
/// * `netmask` - The network mask
///
/// # Returns
///
/// The broadcast address for the network
///
/// # Example
///
/// ```
/// use std::net::Ipv4Addr;
/// use sood::network::calculate_broadcast;
///
/// let ip = Ipv4Addr::new(192, 168, 1, 100);
/// let netmask = Ipv4Addr::new(255, 255, 255, 0);
/// let broadcast = calculate_broadcast(ip, netmask);
///
/// assert_eq!(broadcast, Ipv4Addr::new(192, 168, 1, 255));
/// ```
pub fn calculate_broadcast(ip: Ipv4Addr, netmask: Ipv4Addr) -> Ipv4Addr {
    let ip_bits = u32::from(ip);
    let mask_bits = u32::from(netmask);
    let broadcast_bits = ip_bits | !mask_bits;

    Ipv4Addr::from(broadcast_bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_broadcast() {
        // Class C network
        let ip = Ipv4Addr::new(192, 168, 1, 100);
        let netmask = Ipv4Addr::new(255, 255, 255, 0);
        assert_eq!(
            calculate_broadcast(ip, netmask),
            Ipv4Addr::new(192, 168, 1, 255)
        );

        // Class B network
        let ip = Ipv4Addr::new(172, 16, 5, 10);
        let netmask = Ipv4Addr::new(255, 255, 0, 0);
        assert_eq!(
            calculate_broadcast(ip, netmask),
            Ipv4Addr::new(172, 16, 255, 255)
        );

        // /25 subnet
        let ip = Ipv4Addr::new(192, 168, 1, 50);
        let netmask = Ipv4Addr::new(255, 255, 255, 128);
        assert_eq!(
            calculate_broadcast(ip, netmask),
            Ipv4Addr::new(192, 168, 1, 127)
        );
    }

    #[test]
    fn test_get_network_interfaces() {
        // Just verify it doesn't panic and returns some interfaces
        let interfaces = get_network_interfaces();
        // Most systems will have at least one non-loopback interface
        // but we can't guarantee it in all test environments
        println!("Found {} network interfaces", interfaces.len());
        for iface in interfaces {
            println!("  {}: {} (broadcast: {})", iface.name, iface.ip, iface.broadcast);
        }
    }
}
