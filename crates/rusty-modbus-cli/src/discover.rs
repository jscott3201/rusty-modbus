//! Network discovery — TCP sweep, unit ID probe, device identification.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use ipnet::Ipv4Net;
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient};
use rusty_modbus_types::UnitId;
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HostResult {
    /// Host address.
    pub address: String,
    /// Devices found at this host.
    pub devices: Vec<DiscoveredDevice>,
}

/// Reusable discovery scan report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryReport {
    /// Number of target hosts considered by the scan.
    pub hosts_scanned: usize,
    /// Hosts with an open Modbus port and their responding unit IDs.
    pub results: Vec<HostResult>,
}

/// Run the discovery process.
///
/// # Errors
///
/// Returns an error if arguments are invalid or a fatal I/O error occurs.
pub async fn run(config: DiscoverConfig) -> Result<(), Box<dyn std::error::Error>> {
    let report = scan(&config).await?;

    match config.format {
        OutputFormat::Human => print!("{}", format_human(&report)),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report.results)?),
    }

    Ok(())
}

/// Scan the configured network targets and return structured discovery results.
///
/// # Errors
///
/// Returns an error if arguments are invalid or target host/range parsing fails.
pub async fn scan(config: &DiscoverConfig) -> Result<DiscoveryReport, Box<dyn std::error::Error>> {
    validate_targets(config)?;
    validate_concurrency(config.concurrency)?;
    let timeout = Duration::from_secs(config.timeout);
    let unit_ids = parse_unit_id_range(&config.unit_id_range)?;
    let semaphore = Arc::new(Semaphore::new(config.concurrency));

    let (hosts_scanned, hosts): (usize, Vec<SocketAddr>) = if let Some(ref host) = config.host {
        let ip: IpAddr = host.parse()?;
        (1, vec![SocketAddr::new(ip, config.port)])
    } else if let Some(ref range) = config.range {
        let ips = expand_range(range)?;
        let hosts = sweep_hosts(&ips, config.port, timeout, &semaphore).await;
        (ips.len(), hosts)
    } else {
        return Err("either --range or --host is required".into());
    };

    if hosts.is_empty() {
        return Ok(DiscoveryReport {
            hosts_scanned,
            results: Vec::new(),
        });
    }

    let mut results = Vec::new();
    for addr in &hosts {
        let devices = probe_host(*addr, &unit_ids, timeout, &semaphore).await;
        results.push(HostResult {
            address: addr.to_string(),
            devices,
        });
    }

    Ok(DiscoveryReport {
        hosts_scanned,
        results,
    })
}

fn validate_targets(config: &DiscoverConfig) -> Result<(), Box<dyn std::error::Error>> {
    match (&config.host, &config.range) {
        (Some(_), Some(_)) => Err("use either --host or --range, not both".into()),
        (None, None) => Err("either --range or --host is required".into()),
        _ => Ok(()),
    }
}

fn validate_concurrency(concurrency: usize) -> Result<(), Box<dyn std::error::Error>> {
    if concurrency == 0 {
        Err("--concurrency must be greater than 0".into())
    } else {
        Ok(())
    }
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
            let probe =
                time::timeout(timeout, client.read_holding_registers(UnitId(uid), 0, 1)).await;

            // Any response (success or Modbus exception) means device is present.
            let alive = matches!(probe, Ok(Ok(_)) | Ok(Err(ClientError::Exception(_))));

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
                revision: dev_id.as_ref().and_then(|d| d.major_minor_revision.clone()),
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

/// Render discovery results in the CLI human-readable format.
#[must_use]
pub fn format_human(report: &DiscoveryReport) -> String {
    if report.results.is_empty() {
        return format!(
            "No hosts found with open Modbus port. Scanned {} hosts.\n",
            report.hosts_scanned
        );
    }

    let mut output = String::new();
    let mut total_devices = 0;
    for result in &report.results {
        output.push_str(&result.address);
        output.push('\n');
        if result.devices.is_empty() {
            output.push_str("  No responding unit IDs\n");
        } else {
            for dev in &result.devices {
                total_devices += 1;
                if let (Some(vendor), Some(product), Some(rev)) =
                    (&dev.vendor_name, &dev.product_code, &dev.revision)
                {
                    output.push_str(&format!(
                        "  Unit {:>3}: [{vendor}] {product} rev {rev}\n",
                        dev.unit_id
                    ));
                } else {
                    output.push_str(&format!(
                        "  Unit {:>3}: (no device identification)\n",
                        dev.unit_id
                    ));
                }
            }
        }
    }

    output.push_str(&format!(
        "\nScan complete: {} hosts, {} devices",
        report.results.len(),
        total_devices
    ));
    output.push('\n');
    output
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
        let start_u32 = u32::from(start);
        let end_u32 = u32::from(end);
        if start_u32 > end_u32 {
            return Err(format!("invalid range: start '{start}' is after end '{end}'").into());
        }
        Ok((start_u32..=end_u32).map(Ipv4Addr::from).collect())
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
        if start > end {
            return Err(format!("invalid unit ID range: start {start} is after end {end}").into());
        }
        Ok((start..=end).collect())
    } else {
        let id: u8 = range.parse()?;
        Ok(vec![id])
    }
}

#[cfg(test)]
mod tests {
    use rusty_modbus_sim::{ModbusSimulator, generic_io};

    use super::*;

    fn config_for_host(addr: SocketAddr) -> DiscoverConfig {
        DiscoverConfig {
            range: None,
            host: Some(addr.ip().to_string()),
            port: addr.port(),
            unit_id_range: "1".to_string(),
            timeout: 1,
            concurrency: 4,
            format: OutputFormat::Human,
        }
    }

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
    fn parse_dash_range_rejects_reversed_bounds() {
        let error = expand_range("10.0.0.3-10.0.0.1").unwrap_err();
        assert!(error.to_string().contains("start '10.0.0.3' is after end"));
    }

    #[test]
    fn parse_dash_range_includes_max_ipv4_without_overflow() {
        let ips = expand_range("255.255.255.255-255.255.255.255").unwrap();
        assert_eq!(ips, vec![Ipv4Addr::new(255, 255, 255, 255)]);
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
    fn parse_unit_id_range_rejects_reversed_bounds() {
        let error = parse_unit_id_range("10-1").unwrap_err();
        assert!(error.to_string().contains("start 10 is after end 1"));
    }

    #[test]
    fn parse_single_unit_id() {
        let ids = parse_unit_id_range("5").unwrap();
        assert_eq!(ids, vec![5]);
    }

    #[test]
    fn format_human_reports_no_open_hosts() {
        let report = DiscoveryReport {
            hosts_scanned: 3,
            results: Vec::new(),
        };

        assert_eq!(
            format_human(&report),
            "No hosts found with open Modbus port. Scanned 3 hosts.\n"
        );
    }

    #[test]
    fn format_human_reports_devices_and_missing_identification() {
        let report = DiscoveryReport {
            hosts_scanned: 1,
            results: vec![HostResult {
                address: "127.0.0.1:502".to_string(),
                devices: vec![
                    DiscoveredDevice {
                        unit_id: 1,
                        vendor_name: Some("ACME".to_string()),
                        product_code: Some("IO".to_string()),
                        revision: Some("1.0".to_string()),
                    },
                    DiscoveredDevice {
                        unit_id: 2,
                        vendor_name: None,
                        product_code: None,
                        revision: None,
                    },
                ],
            }],
        };

        let output = format_human(&report);
        assert!(output.contains("127.0.0.1:502"));
        assert!(output.contains("Unit   1: [ACME] IO rev 1.0"));
        assert!(output.contains("Unit   2: (no device identification)"));
        assert!(output.contains("Scan complete: 1 hosts, 2 devices"));
    }

    #[tokio::test]
    async fn scan_rejects_zero_concurrency() {
        let mut config = config_for_host("127.0.0.1:502".parse().unwrap());
        config.concurrency = 0;

        let error = scan(&config).await.unwrap_err();
        assert!(error.to_string().contains("concurrency"));
    }

    #[tokio::test]
    async fn scan_rejects_host_and_range_together() {
        let mut config = config_for_host("127.0.0.1:502".parse().unwrap());
        config.range = Some("127.0.0.1".to_string());

        let error = scan(&config).await.unwrap_err();
        assert!(error.to_string().contains("either --host or --range"));
    }

    #[tokio::test]
    async fn scan_rejects_missing_target() {
        let mut config = config_for_host("127.0.0.1:502".parse().unwrap());
        config.host = None;

        let error = scan(&config).await.unwrap_err();
        assert!(error.to_string().contains("either --range or --host"));
    }

    #[tokio::test]
    async fn scan_host_finds_simulator_unit() {
        let mut sim = ModbusSimulator::from_config(generic_io()).unwrap();
        let addr = sim.start().await.unwrap();

        let report = scan(&config_for_host(addr)).await.unwrap();

        assert_eq!(report.hosts_scanned, 1);
        assert_eq!(report.results.len(), 1);
        assert_eq!(report.results[0].address, addr.to_string());
        assert_eq!(report.results[0].devices[0].unit_id, 1);

        sim.stop().await;
    }
}
