//! Example: Discover Roon devices on the local network
//!
//! This example demonstrates how to use the Sood library to discover
//! devices (like Roon Core) on the local network.
//!
//! Run with: cargo run --example discover

use sood::Sood;
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
    let mut messages = sood.messages();

    println!("Starting discovery (listening on all network interfaces)...");
    sood.start().await?;

    // Give it a moment to set up all interfaces
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send a discovery query for Roon services
    println!("Sending discovery query for Roon services...\n");
    let service_id = "00720724-5143-4a9b-abac-0e50cba674bb";

    sood.discover_service(service_id).await?;

    // Listen for responses
    println!("Listening for responses... (Press Ctrl+C to exit)\n");
    let mut discovered_devices = std::collections::HashSet::new();

    // Set up interval for periodic queries
    let mut query_interval = tokio::time::interval(Duration::from_secs(10));
    query_interval.tick().await; // Skip first immediate tick

    loop {
        tokio::select! {
            result = messages.recv() => {
                match result {
                    Ok(msg) => {
                        // Filter for actual device responses (not our own queries)
                        let is_device_response = msg.properties.get("service_id")
                            .and_then(|v| v.as_ref())
                            .map(|v| v == "00720724-5143-4a9b-abac-0e50cba674bb")
                            .unwrap_or(false)
                            && msg.properties.contains_key("unique_id");

                        if is_device_response {
                            // Use unique_id for tracking, not IP address
                            let unique_id = msg.properties.get("unique_id")
                                .and_then(|v| v.as_ref())
                                .map(|s| s.as_str())
                                .unwrap_or("unknown");

                            if discovered_devices.insert(msg.from) {
                                println!("╔══════════════════════════════════════════════════");
                                println!("║ 🎯 SOOD Device Found!");
                                println!("╠══════════════════════════════════════════════════");
                                println!("║ Address: {}", msg.from);
                                println!("║ Unique ID: {}", unique_id);

                                // Display HTTP port if available
                                if let Some(Some(port)) = msg.properties.get("http_port") {
                                    println!("║ HTTP Port: {}", port);
                                    println!("║ Connect: http://{}:{}", msg.from.ip(), port);
                                }

                                println!("║");
                                println!("║ All Properties:");
                                for (key, value) in &msg.properties {
                                    match value {
                                        Some(val) => println!("║   {}: {}", key, val),
                                        None => println!("║   {}: null", key),
                                    }
                                }
                                println!("╚══════════════════════════════════════════════════\n");
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error receiving message: {}", e);
                        break;
                    }
                }
            }
            _ = query_interval.tick() => {
                println!("Sending periodic discovery query...");
                if let Err(e) = sood.discover_service(service_id).await {
                    eprintln!("Failed to send query: {}", e);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down...");
                break;
            }
        }
    }

    // Clean shutdown
    println!("Discovered {} unique device(s)", discovered_devices.len());
    sood.stop().await;
    println!("Goodbye!");

    Ok(())
}
