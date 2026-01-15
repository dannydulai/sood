# Sood - Service-Oriented Object Discovery

A Rust implementation of the Sood discovery protocol used by Roon for discovering the Core as well as audio endpoints on the local network.

## Overview

Sood is a UDP-based service discovery protocol that uses multicast and broadcast to discover devices on the local network. This crate provides a simple async API for sending discovery queries and receiving responses.

## Features

- **Async/await API** using Tokio
- **Automatic network interface monitoring** - detects when interfaces are added or removed
- **Multicast and broadcast discovery** - sends queries via both methods for maximum compatibility
- **Handles network changes gracefully** - adapts to changing network conditions
- **Transaction ID auto-generation** - automatically assigns unique IDs to queries
- **Multiple subscribers** - supports multiple message receivers simultaneously

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
sood = "0.1"
tokio = { version = "1.0", features = ["full"] }
```

## Quick Start

```rust
use sood::Sood;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Create and start the discovery client
    let mut sood = Sood::new()?;
    let mut messages = sood.messages();

    sood.start().await?;

    // Send a discovery query for Roon services
    sood.discover_service("com.roon.app").await?;

    // Listen for responses
    while let Ok(msg) = messages.recv().await {
        println!("Discovered device at {}", msg.from);
        println!("  Properties: {:?}", msg.properties);
    }

    Ok(())
}
```

## Protocol Details

### Network Parameters

- **Multicast Address**: 239.255.90.90
- **Port**: 9003
- **Protocol Version**: 2
- **Multicast TTL**: 1 (local network only)

### Message Format

SOOD messages follow this binary format:

```
+----------------+----------------+
| "SOOD" (4B)    | Version (1B)   |
+----------------+----------------+
| Type (1B)      | Properties...  |
+----------------+----------------+
```

Properties are encoded as:
- Key: 1 byte length + UTF-8 string
- Value: 2 bytes length (big-endian) + UTF-8 string
- Special: length 0xFFFF represents a null value

### Special Properties

- `_tid`: Transaction ID (auto-generated if not provided)
- `_replyaddr`: Override source IP address in responses
- `_replyport`: Override source port in responses

### Query vs Response Messages

When discovering devices:

**Query messages** (what you send):
- Contain `query_service_id` property
- Broadcast to all devices on the network
- Example: `{ "query_service_id": "00720724-5143-4a9b-abac-0e50cba674bb" }`

**Response messages** (what devices send back):
- Contain `service_id` (not `query_service_id`)
- Contain `unique_id` - unique identifier for the device
- Contain `http_port` - port for HTTP/WebSocket connection
- May contain additional device-specific properties

You'll also receive echoes of your own query messages on multicast interfaces - filter these out by checking for `service_id` and `unique_id` properties.

## Examples

### Basic Discovery

```rust
use sood::Sood;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut sood = Sood::new()?;
    let mut messages = sood.messages();

    sood.start().await?;

    // Simple discovery by service ID
    sood.discover_service("00720724-5143-4a9b-abac-0e50cba674bb").await?;

    // Listen for responses
    while let Ok(msg) = messages.recv().await {
        // Filter for actual device responses (not query echoes)
        let is_device = msg.properties.get("service_id")
            .and_then(|v| v.as_ref())
            .map(|v| v == "00720724-5143-4a9b-abac-0e50cba674bb")
            .unwrap_or(false)
            && msg.properties.contains_key("unique_id");

        if is_device {
            let unique_id = msg.properties.get("unique_id")
                .and_then(|v| v.as_ref())
                .unwrap();
            let http_port = msg.properties.get("http_port")
                .and_then(|v| v.as_ref())
                .unwrap();

            println!("Device {} at {}:{}", unique_id, msg.from.ip(), http_port);
        }
    }

    Ok(())
}
```

### Advanced Queries

For queries with multiple properties, use the `query()` method:

```rust
use sood::Sood;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut sood = Sood::new()?;
    sood.start().await?;

    // Complex query with multiple properties
    let mut props = HashMap::new();
    props.insert("query_service_id".to_string(), Some("00720724-5143-4a9b-abac-0e50cba674bb".to_string()));
    props.insert("version".to_string(), Some("1.0".to_string()));
    sood.query(props).await?;

    Ok(())
}
```

### Continuous Discovery with Periodic Queries

```rust
use sood::Sood;
use std::time::Duration;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut sood = Sood::new()?;
    let mut messages = sood.messages();

    sood.start().await?;

    let service_id = "com.roon.app";

    // Send queries every 10 seconds
    let sood_ref = &sood;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            let _ = sood_ref.discover_service(service_id).await;
        }
    });

    // Listen continuously
    while let Ok(msg) = messages.recv().await {
        println!("Device: {} - {:?}", msg.from, msg.properties);
    }

    Ok(())
}
```

### Multiple Message Receivers

```rust
use sood::Sood;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut sood = Sood::new()?;

    // Create multiple independent receivers
    let mut rx1 = sood.messages();
    let mut rx2 = sood.messages();

    sood.start().await?;

    // Process messages in separate tasks
    tokio::spawn(async move {
        while let Ok(msg) = rx1.recv().await {
            println!("Handler 1: {}", msg.from);
        }
    });

    tokio::spawn(async move {
        while let Ok(msg) = rx2.recv().await {
            println!("Handler 2: {}", msg.from);
        }
    });

    // Keep running...
    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

## Running the Example

This crate includes a complete example that demonstrates device discovery:

```bash
cd sood
cargo run --example discover
```

The example will:
1. Start listening on all network interfaces
2. Send periodic discovery queries
3. Display discovered devices with their properties
4. Handle Ctrl+C for clean shutdown

## Architecture

The library is organized into several modules:

- **`protocol`**: Message parsing and serialization
- **`network`**: Network interface enumeration and utilities
- **`discovery`**: Core discovery engine with socket management
- **`lib`**: Public API

### How It Works

1. **Initialization**: Creates sockets for each network interface
2. **Multicast Listening**: Binds to port 9003 and joins multicast group on each interface
3. **Broadcast Sending**: Sends queries to both multicast and broadcast addresses
4. **Interface Monitoring**: Checks for interface changes every 5 seconds
5. **Message Distribution**: Uses Tokio broadcast channels to distribute received messages

## Platform Support

This library works on:
- Linux
- macOS
- Windows

Network interface monitoring and multicast support may vary by platform.

## Logging

The library uses the `tracing` crate for logging. Enable logging in your application:

```rust
tracing_subscriber::fmt()
    .with_max_level(tracing::Level::INFO)
    .init();
```

## Performance Considerations

- Each network interface requires 2 UDP sockets (one for receiving, one for sending)
- The library uses broadcast channels with a default capacity of 100 messages
- Interface checks occur every 5 seconds in a background task
- All I/O is non-blocking and uses Tokio for async operations

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## References

- [Roon](https://roonlabs.com/) - The audio player that uses this protocol
- Original Node.js implementation: Based on the reference implementation

## Troubleshooting

### No devices discovered

- Check that your firewall allows UDP traffic on port 9003
- Verify multicast is enabled on your network interfaces
- Ensure devices are on the same local network
- Run with `RUST_LOG=sood=debug` to see detailed logging

### Permission errors

On some systems, binding to multicast addresses may require elevated privileges:

```bash
sudo cargo run --example discover
```

### Interface changes not detected

The library checks for interface changes every 5 seconds. If you need faster detection, this can be adjusted in the source code.
