//! SOOD protocol message parsing and serialization.
//!
//! This module implements the SOOD (Service-Oriented Object Discovery) protocol
//! message format used by Roon for device discovery.
//!
//! ## Message Format
//!
//! - Header: "SOOD" (4 bytes)
//! - Version: 2 (1 byte)
//! - Type: "Q" for query (1 byte)
//! - Properties: Variable-length key-value pairs
//!   - Key: 1 byte length + UTF-8 string
//!   - Value: 2 bytes length (big-endian) + UTF-8 string
//!   - Special: length 0xFFFF represents a null value

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

const SOOD_HEADER: &[u8] = b"SOOD";
const SOOD_VERSION: u8 = 2;
const NULL_VALUE_MARKER: u16 = 0xFFFF;

/// Type of SOOD message
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    /// Query message
    Query,
    /// Response message
    Response,
}

impl MessageType {
    fn as_byte(&self) -> u8 {
        match self {
            MessageType::Query => b'Q',
            MessageType::Response => b'R',
        }
    }

    fn from_byte(b: u8) -> Option<Self> {
        match b {
            b'Q' => Some(MessageType::Query),
            b'R' => Some(MessageType::Response),
            _ => None,
        }
    }
}

/// A parsed SOOD message
#[derive(Debug, Clone)]
pub struct Message {
    /// The source address of the message
    pub from: SocketAddr,
    /// The message type
    pub msg_type: MessageType,
    /// Message properties as key-value pairs (None represents null)
    pub properties: HashMap<String, Option<String>>,
}

/// Parse a SOOD message from a UDP packet
///
/// Returns `None` if the message is invalid or doesn't follow the SOOD protocol.
///
/// # Special Properties
///
/// - `_replyaddr`: Overrides the source IP address
/// - `_replyport`: Overrides the source port
pub fn parse_message(buf: &[u8], mut from: SocketAddr) -> Option<Message> {
    // Minimum message size: SOOD(4) + version(1) + type(1) = 6 bytes
    if buf.len() < 6 {
        tracing::debug!("Message too short: {} bytes", buf.len());
        return None;
    }

    // Validate header
    if &buf[0..4] != SOOD_HEADER {
        tracing::debug!("Invalid header: {:?}", &buf[0..4]);
        return None;
    }

    // Validate version
    if buf[4] != SOOD_VERSION {
        tracing::debug!("Invalid version: {}", buf[4]);
        return None;
    }

    // Parse message type
    let msg_type = MessageType::from_byte(buf[5])?;
    tracing::debug!("Parsing message type: {:?} from {}", msg_type, from);

    // Parse properties
    let mut properties = HashMap::new();
    let mut pos = 6;

    while pos < buf.len() {
        // Read key length
        let key_len = buf[pos] as usize;
        pos += 1;

        if key_len == 0 {
            return None; // Invalid key length
        }

        // Check bounds for key
        if pos + key_len > buf.len() {
            return None;
        }

        // Read key
        let key = String::from_utf8(buf[pos..pos + key_len].to_vec()).ok()?;
        pos += key_len;

        // Check bounds for value length
        if pos + 2 > buf.len() {
            return None;
        }

        // Read value length (big-endian)
        let val_len = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
        pos += 2;

        // Read value
        let value = if val_len == NULL_VALUE_MARKER {
            None
        } else if val_len == 0 {
            Some(String::new())
        } else {
            let val_len = val_len as usize;
            if pos + val_len > buf.len() {
                return None;
            }
            let val = String::from_utf8(buf[pos..pos + val_len].to_vec()).ok()?;
            pos += val_len;
            Some(val)
        };

        properties.insert(key, value);
    }

    // Handle special properties that override the source address
    if let Some(Some(addr)) = properties.remove("_replyaddr") {
        if let Ok(ip) = addr.parse::<Ipv4Addr>() {
            from.set_ip(IpAddr::V4(ip));
        }
    }

    if let Some(Some(port_str)) = properties.remove("_replyport") {
        if let Ok(port) = port_str.parse::<u16>() {
            from.set_port(port);
        }
    }

    Some(Message {
        from,
        msg_type,
        properties,
    })
}

/// Serialize a SOOD query message
///
/// Automatically generates a `_tid` (transaction ID) if not present in properties.
///
/// # Arguments
///
/// * `properties` - Key-value pairs to include in the query
///
/// # Returns
///
/// A buffer containing the serialized SOOD query message
pub fn serialize_query(properties: &mut HashMap<String, Option<String>>) -> Vec<u8> {
    // Auto-generate transaction ID if not present
    if !properties.contains_key("_tid") {
        properties.insert("_tid".to_string(), Some(uuid::Uuid::new_v4().to_string()));
    }

    // Estimate buffer size (conservative upper bound)
    let mut buf = Vec::with_capacity(65535);

    // Write header
    buf.extend_from_slice(SOOD_HEADER);
    buf.push(SOOD_VERSION);
    buf.push(MessageType::Query.as_byte());

    // Write properties
    for (key, value) in properties.iter() {
        let key_bytes = key.as_bytes();
        let key_len = key_bytes.len().min(255); // Max key length is 255

        // Write key length and key
        buf.push(key_len as u8);
        buf.extend_from_slice(&key_bytes[..key_len]);

        // Write value length and value
        match value {
            None => {
                // Null value
                buf.extend_from_slice(&NULL_VALUE_MARKER.to_be_bytes());
            }
            Some(val) => {
                let val_bytes = val.as_bytes();
                let val_len = val_bytes.len().min(65534); // Max value length (0xFFFF is reserved)

                buf.extend_from_slice(&(val_len as u16).to_be_bytes());
                buf.extend_from_slice(&val_bytes[..val_len]);
            }
        }
    }

    buf
}

/// Serialize a SOOD response message
///
/// Similar to serialize_query but for response messages.
///
/// # Arguments
///
/// * `properties` - Key-value pairs to include in the response
///
/// # Returns
///
/// A buffer containing the serialized SOOD response message
pub fn serialize_response(properties: &mut HashMap<String, Option<String>>) -> Vec<u8> {
    // Estimate buffer size (conservative upper bound)
    let mut buf = Vec::with_capacity(65535);

    // Write header
    buf.extend_from_slice(SOOD_HEADER);
    buf.push(SOOD_VERSION);
    buf.push(MessageType::Response.as_byte());

    // Write properties
    for (key, value) in properties.iter() {
        let key_bytes = key.as_bytes();
        let key_len = key_bytes.len().min(255); // Max key length is 255

        // Write key length and key
        buf.push(key_len as u8);
        buf.extend_from_slice(&key_bytes[..key_len]);

        // Write value length and value
        match value {
            None => {
                // Null value
                buf.extend_from_slice(&NULL_VALUE_MARKER.to_be_bytes());
            }
            Some(val) => {
                let val_bytes = val.as_bytes();
                let val_len = val_bytes.len().min(65534); // Max value length (0xFFFF is reserved)

                buf.extend_from_slice(&(val_len as u16).to_be_bytes());
                buf.extend_from_slice(&val_bytes[..val_len]);
            }
        }
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_query() {
        // Create a simple query message
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(2); // version
        buf.push(b'Q'); // type

        // Add property: "key" = "value"
        buf.push(3); // key length
        buf.extend_from_slice(b"key");
        buf.extend_from_slice(&5u16.to_be_bytes()); // value length
        buf.extend_from_slice(b"value");

        let from = "192.168.1.100:9003".parse().unwrap();
        let msg = parse_message(&buf, from).unwrap();

        assert_eq!(msg.msg_type, MessageType::Query);
        assert_eq!(msg.properties.get("key"), Some(&Some("value".to_string())));
    }

    #[test]
    fn test_parse_null_value() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(2);
        buf.push(b'Q');

        // Add property: "key" = null
        buf.push(3);
        buf.extend_from_slice(b"key");
        buf.extend_from_slice(&0xFFFFu16.to_be_bytes());

        let from = "192.168.1.100:9003".parse().unwrap();
        let msg = parse_message(&buf, from).unwrap();

        assert_eq!(msg.properties.get("key"), Some(&None));
    }

    #[test]
    fn test_serialize_query() {
        let mut props = HashMap::new();
        props.insert("service".to_string(), Some("roon".to_string()));

        let buf = serialize_query(&mut props);

        // Verify header
        assert_eq!(&buf[0..4], b"SOOD");
        assert_eq!(buf[4], 2);
        assert_eq!(buf[5], b'Q');

        // Verify it can be parsed back
        let from = "192.168.1.100:9003".parse().unwrap();
        let msg = parse_message(&buf, from).unwrap();

        assert_eq!(msg.msg_type, MessageType::Query);
        assert_eq!(msg.properties.get("service"), Some(&Some("roon".to_string())));
        assert!(msg.properties.contains_key("_tid")); // Auto-generated
    }
}
