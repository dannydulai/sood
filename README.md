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
use sood::{service_ids, Sood};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Create and start the discovery client
    let mut sood = Sood::new()?;
    sood.start().await?;

    // Get a receiver for discovered devices (includes past discoveries)
    let mut devices = sood.discovered().await;

    // Send a discovery query for Roon API services
    sood.discover_service(service_ids::ROON_API).await?;

    // Listen for discovered devices (automatically filtered and deduplicated)
    while let Ok(device) = devices.recv().await {
        println!("Discovered: {} at {}", device.unique_id, device.from);
        if let Some(Some(port)) = device.properties.get("http_port") {
            println!("  Connect: http://{}:{}", device.from.ip(), port);
        }
    }

    Ok(())
}
```

## Well-Known Service IDs

The library provides constants for common Roon service IDs:

- **`service_ids::ROON_API`** - Roon API service (used by extensions and control applications)
- **`service_ids::ROON_SERVER`** - Roon Core server
- **`service_ids::ROON_OS`** - Roon OS/Nucleus devices

You can use these instead of hardcoding UUIDs:

```rust
use sood::{service_ids, Sood};

// Discover Roon API services
sood.discover_service(service_ids::ROON_API).await?;

// Discover Roon Core servers
sood.discover_service(service_ids::ROON_SERVER).await?;

// Discover Roon OS devices
sood.discover_service(service_ids::ROON_OS).await?;
```

## Examples

### Basic Discovery

```rust
use sood::{service_ids, Sood};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut sood = Sood::new()?;
    sood.start().await?;

    // Get receiver - no race condition, includes past discoveries
    let mut devices = sood.discovered().await;

    // Simple discovery by service ID
    sood.discover_service(service_ids::ROON_API).await?;

    // Listen for devices - automatically filtered and deduplicated!
    while let Ok(device) = devices.recv().await {
        println!("Device {} at {}", device.unique_id, device.from.ip());

        if let Some(Some(port)) = device.properties.get("http_port") {
            println!("  HTTP: http://{}:{}", device.from.ip(), port);
        }
    }

    Ok(())
}
```

### Getting All Discovered Devices

Get a snapshot of all devices discovered so far:

```rust
use sood::{service_ids, Sood};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut sood = Sood::new()?;
    sood.start().await?;

    sood.discover_service(service_ids::ROON_API).await?;

    // Wait a bit for responses
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Get all discovered devices
    let devices = sood.get_discovered_devices().await;
    println!("Found {} devices:", devices.len());
    for device in devices {
        println!("  - {} at {}", device.unique_id, device.from);
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

### Raw Message Access

For advanced use cases, you can access raw messages (responses only, query reflections are filtered out):

```rust
use sood::{service_ids, Sood};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut sood = Sood::new()?;
    let mut messages = sood.messages();

    sood.start().await?;
    sood.discover_service(service_ids::ROON_API).await?;

    // Process all response messages (including non-device responses)
    while let Ok(msg) = messages.recv().await {
        println!("Message from {}: {:?}", msg.from, msg.properties);
    }

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
    sood.start().await?;

    let mut devices = sood.discovered().await;
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

    // Listen continuously - devices are automatically deduplicated
    while let Ok(device) = devices.recv().await {
        println!("Device: {} at {}", device.unique_id, device.from);
    }

    Ok(())
}
```

### Multiple Device Receivers

You can create multiple independent receivers that all get the same devices:

```rust
use sood::{service_ids, Sood};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut sood = Sood::new()?;
    sood.start().await?;

    // Create multiple independent receivers
    let mut rx1 = sood.discovered().await;
    let mut rx2 = sood.discovered().await;

    sood.discover_service(service_ids::ROON_API).await?;

    // Process devices in separate tasks
    tokio::spawn(async move {
        while let Ok(device) = rx1.recv().await {
            println!("Handler 1: {}", device.unique_id);
        }
    });

    tokio::spawn(async move {
        while let Ok(device) = rx2.recv().await {
            println!("Handler 2: {}", device.unique_id);
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
