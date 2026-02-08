//! Example: Discover Roon devices on the local network
//!
//! This example demonstrates how to use the Sood library to discover
//! devices (like Roon Core) on the local network.
//!
//! Run with: cargo run --example discover

use sood::{service_ids, Sood};
use std::time::Duration;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Initialize logging to see what's happening
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("Sood Discovery Example");
    println!("======================\n");

    // Create and start the discovery client
    println!("Creating Sood discovery client...");
    let mut sood = Sood::new()?;

    println!("Starting discovery (listening on all network interfaces)...");
    sood.start().await?;

    // Get discovered device receiver (includes any devices found before this call)
    let mut devices = sood.discovered().await;

    // Give it a moment to set up all interfaces
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send a discovery query for Roon services
    println!("Sending discovery query for Roon services...\n");
    sood.discover_service(service_ids::ROON_API).await?;

    // Listen for devices
    println!("Listening for devices... (Press Ctrl+C to exit)\n");

    loop {
        tokio::select! {
            result = devices.recv() => {
                match result {
                    Ok(device) => {
                        println!("╔══════════════════════════════════════════════════");
                        println!("║ 🎯 Device Found!");
                        println!("╠══════════════════════════════════════════════════");
                        println!("║ Address: {}", device.from);
                        println!("║");
                        for (key, value) in &device.properties {
                            match value {
                                Some(val) => println!("║ {}: {}", key, val),
                                None => println!("║ {}: null", key),
                            }
                        }
                        println!("╚══════════════════════════════════════════════════\n");
                    }
                    Err(e) => {
                        eprintln!("Error receiving device: {}", e);
                        break;
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down...");
                break;
            }
        }
    }

    // Show final count
    let all_devices = sood.get_discovered_devices().await;
    println!("Discovered {} unique device(s)", all_devices.len());
    sood.stop().await;
    println!("Goodbye!");

    Ok(())
}
