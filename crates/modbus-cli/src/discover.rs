//! Network discovery — TCP sweep, unit ID probe, device identification.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use ipnet::Ipv4Net;
use modbus_client::{ClientConfig, ClientError, ModbusClient};
use modbus_types::UnitId;
use tokio::sync::Semaphore;
use tokio::time;

use crate::output::OutputFormat;

/// Discover configuration from CLI args.
pub struct DiscoverConfig {
    /// CIDR range to scan.
    pub range: Option<String>,
    /// Single host to probe.
    pub host: Option<String>,
    /// Target port.
    pub port: u16,
    /// Unit ID range string (e.g., "1-247").
    pub unit_id_range: String,
    /// Per-probe timeout in seconds.
    pub timeout: u64,
    /// Max concurrent connections.
    pub concurrency: usize,
    /// Output format.
    pub format: OutputFormat,
}

/// A discovered device.
#[derive(Debug, serde::Serialize)]
pub struct DiscoveredDevice {
    /// Unit ID that responded.
    pub unit_id: u8,
    /// Vendor name from device identification (if available).
    pub vendor_name: Option<String>,
    /// Product code from device identification (if available).
    pub product_code: Option<String>,
    /// Revision from device identification (if available).
    pub revision: Option<String>,
}

/// Result for a single host.
#[derive(Debug, serde::Serialize)]
pub struct HostResult {
    /// Host address.
    pub address: String,
    /// Devices found at this host.
    pub devices: Vec<DiscoveredDevice>,
}

/// Run the discovery process.
///
/// # Errors
///
/// Returns an error if arguments are invalid or a fatal I/O error occurs.
pub async fn run(config: DiscoverConfig) -> Result<(), Box<dyn std::error::Error>> {
    let timeout = Duration::from_secs(config.timeout);
    let unit_ids = parse_unit_id_range(&config.unit_id_range)?;
    let semaphore = Arc::new(Semaphore::new(config.concurrency));

    // Phase 1: Determine target hosts.
    let hosts: Vec<SocketAddr> = if let Some(ref host) = config.host {
        let ip: IpAddr = host.parse()?;
        vec![SocketAddr::new(ip, config.port)]
    } else if let Some(ref range) = config.range {
        let ips = expand_range(range)?;
        if config.format == OutputFormat::Human {
            eprintln!("Scanning {} hosts on port {}...", ips.len(), config.port);
        }
        sweep_hosts(&ips, config.port, timeout, &semaphore).await
    } else {
        return Err("either --range or --host is required".into());
    };

    if hosts.is_empty() {
        if config.format == OutputFormat::Human {
            println!("No hosts found with open Modbus port.");
        } else {
            println!("[]");
        }
        return Ok(());
    }

    if config.format == OutputFormat::Human {
        eprintln!("Found {} hosts with open Modbus port\n", hosts.len());
    }

    // Phase 2 + 3: Probe unit IDs and get device identification.
    let mut results = Vec::new();
    for addr in &hosts {
        let devices = probe_host(*addr, &unit_ids, timeout, &semaphore).await;
        results.push(HostResult {
            address: addr.to_string(),
            devices,
        });
    }

    // Output results.
    match config.format {
        OutputFormat::Human => print_human(&results),
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
    }

    Ok(())
}

/// Phase 1: TCP connect sweep to find open Modbus ports.
async fn sweep_hosts(
    ips: &[Ipv4Addr],
    port: u16,
    timeout: Duration,
    semaphore: &Arc<Semaphore>,
) -> Vec<SocketAddr> {
    let mut handles = Vec::new();
    for &ip in ips {
        let addr = SocketAddr::new(IpAddr::V4(ip), port);
        let sem = Arc::clone(semaphore);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.ok()?;
            match time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await {
                Ok(Ok(_)) => Some(addr),
                _ => None,
            }
        }));
    }

    let mut found = Vec::new();
    for h in handles {
        if let Ok(Some(addr)) = h.await {
            found.push(addr);
        }
    }
    found.sort();
    found
}

/// Phase 2 + 3: Probe unit IDs on a single host, then attempt device identification.
async fn probe_host(
    addr: SocketAddr,
    unit_ids: &[u8],
    timeout: Duration,
    semaphore: &Arc<Semaphore>,
) -> Vec<DiscoveredDevice> {
    let per_host_sem = Arc::new(Semaphore::new(16));
    let mut handles = Vec::new();

    for &uid in unit_ids {
        let host_sem = Arc::clone(&per_host_sem);
        let global_sem = Arc::clone(semaphore);
        handles.push(tokio::spawn(async move {
            let _host_permit = host_sem.acquire().await.ok()?;
            let _global_permit = global_sem.acquire().await.ok()?;

            let config = ClientConfig {
                unit_id: UnitId(uid),
                timeout,
                ..ClientConfig::default()
            };

            let client = time::timeout(timeout, ModbusClient::connect(addr, config))
                .await
                .ok()?
                .ok()?;

            // Probe: try reading 1 holding register.
            let probe = time::timeout(
                timeout,
                client.read_holding_registers(UnitId(uid), 0, 1),
            )
            .await;

            // Any response (success or Modbus exception) means device is present.
            let alive = matches!(
                probe,
                Ok(Ok(_)) | Ok(Err(ClientError::Exception(_)))
            );

            if !alive {
                return None;
            }

            // Phase 3: Attempt device identification (best-effort).
            let dev_id = time::timeout(timeout, client.read_device_identification(UnitId(uid)))
                .await
                .ok()
                .and_then(|r| r.ok());

            Some(DiscoveredDevice {
                unit_id: uid,
                vendor_name: dev_id.as_ref().and_then(|d| d.vendor_name.clone()),
                product_code: dev_id.as_ref().and_then(|d| d.product_code.clone()),
                revision: dev_id
                    .as_ref()
                    .and_then(|d| d.major_minor_revision.clone()),
            })
        }));
    }

    let mut devices = Vec::new();
    for h in handles {
        if let Ok(Some(dev)) = h.await {
            devices.push(dev);
        }
    }
    devices.sort_by_key(|d| d.unit_id);
    devices
}

fn print_human(results: &[HostResult]) {
    let mut total_devices = 0;
    for result in results {
        println!("{}", result.address);
        if result.devices.is_empty() {
            println!("  No responding unit IDs");
        } else {
            for dev in &result.devices {
                total_devices += 1;
                if let (Some(vendor), Some(product), Some(rev)) =
                    (&dev.vendor_name, &dev.product_code, &dev.revision)
                {
                    println!("  Unit {:>3}: [{vendor}] {product} rev {rev}", dev.unit_id);
                } else {
                    println!("  Unit {:>3}: (no device identification)", dev.unit_id);
                }
            }
        }
    }
    println!(
        "\nScan complete: {} hosts, {} devices",
        results.len(),
        total_devices
    );
}

/// Expand a CIDR notation or dash-range into a list of IPv4 addresses.
///
/// # Errors
///
/// Returns an error if the range string cannot be parsed.
pub fn expand_range(range: &str) -> Result<Vec<Ipv4Addr>, Box<dyn std::error::Error>> {
    if range.contains('/') {
        let net: Ipv4Net = range.parse()?;
        Ok(net.hosts().collect())
    } else if range.contains('-') {
        let parts: Vec<&str> = range.splitn(2, '-').collect();
        if parts.len() != 2 {
            return Err(format!("invalid range: '{range}'").into());
        }
        let start: Ipv4Addr = parts[0].parse()?;
        let end: Ipv4Addr = parts[1].parse()?;
        let mut ips = Vec::new();
        let mut current = u32::from(start);
        let end_u32 = u32::from(end);
        while current <= end_u32 {
            ips.push(Ipv4Addr::from(current));
            current += 1;
        }
        Ok(ips)
    } else {
        let ip: Ipv4Addr = range.parse()?;
        Ok(vec![ip])
    }
}

/// Parse a unit ID range like "1-247" or "5".
///
/// # Errors
///
/// Returns an error if the range string cannot be parsed.
pub fn parse_unit_id_range(range: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if range.contains('-') {
        let parts: Vec<&str> = range.splitn(2, '-').collect();
        let start: u8 = parts[0].parse()?;
        let end: u8 = parts[1].parse()?;
        Ok((start..=end).collect())
    } else {
        let id: u8 = range.parse()?;
        Ok(vec![id])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cidr_range() {
        let ips = expand_range("192.168.1.0/30").unwrap();
        // hosts() excludes network (.0) and broadcast (.3)
        assert_eq!(ips.len(), 2);
    }

    #[test]
    fn parse_dash_range() {
        let ips = expand_range("10.0.0.1-10.0.0.3").unwrap();
        assert_eq!(ips.len(), 3);
    }

    #[test]
    fn parse_single_ip() {
        let ips = expand_range("192.168.1.1").unwrap();
        assert_eq!(ips.len(), 1);
        assert_eq!(ips[0], Ipv4Addr::new(192, 168, 1, 1));
    }

    #[test]
    fn parse_unit_id_range_dash() {
        let ids = parse_unit_id_range("1-10").unwrap();
        assert_eq!(ids, (1..=10).collect::<Vec<u8>>());
    }

    #[test]
    fn parse_single_unit_id() {
        let ids = parse_unit_id_range("5").unwrap();
        assert_eq!(ids, vec![5]);
    }
}
