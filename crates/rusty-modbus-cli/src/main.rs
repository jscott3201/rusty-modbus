//! Modbus CLI tool — read/write registers and coils from the command line.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient};
use rusty_modbus_types::UnitId;

mod commands;
mod dashboard;
mod discover;
mod logging;
mod output;
mod server;
mod shell;
mod shell_parser;

/// Modbus command-line diagnostic tool.
#[derive(Parser, Debug)]
#[command(name = "modbus", version, about = "Modbus CLI diagnostic tool")]
struct Cli {
    /// Target host (host/IP with optional port — port defaults to 502).
    #[arg(long, short = 'H', global = true)]
    host: Option<String>,

    /// Target port. Default: 502.
    #[arg(long, short = 'p', global = true, default_value_t = 502)]
    port: u16,

    /// Modbus unit/slave ID. Defaults to 255 for clients, 1 for `server`.
    #[arg(long, short = 'u', global = true)]
    unit_id: Option<u8>,

    /// Request timeout in seconds. Default: 5.
    #[arg(long, short = 't', global = true, default_value_t = 5)]
    timeout: u64,

    /// Output format.
    #[arg(long, global = true, default_value = "human")]
    format: output::OutputFormat,

    /// Tracing filter for diagnostic logs. Defaults to RUST_LOG or "warn".
    #[arg(long, global = true, value_name = "FILTER")]
    log_filter: Option<String>,

    /// Diagnostic log format.
    #[arg(long, global = true, value_enum, default_value = "human")]
    log_format: logging::LogFormat,

    /// Write diagnostic logs to a file instead of stderr.
    #[arg(long, global = true, value_name = "PATH")]
    log_file: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Read registers, coils, or discrete inputs.
    Read(commands::ReadArgs),
    /// Write registers or coils.
    Write(commands::WriteArgs),
    /// Interactive Modbus shell.
    Shell,
    /// Interactive terminal dashboard.
    Dashboard(dashboard::DashboardArgs),
    /// Run an in-memory Modbus/TCP server.
    Server(server::ServerArgs),
    /// Discover Modbus devices on the network.
    Discover(DiscoverArgs),
}

/// Arguments for the `discover` subcommand.
#[derive(clap::Args, Debug)]
struct DiscoverArgs {
    /// CIDR range to scan (e.g., 192.168.1.0/24).
    #[arg(long)]
    range: Option<String>,

    /// Unit ID range to probe (e.g., 1-247). Default: 1-247.
    #[arg(long, default_value = "1-247")]
    unit_ids: String,

    /// Per-probe timeout in seconds. Default: 2.
    #[arg(long, default_value_t = 2)]
    discover_timeout: u64,

    /// Max concurrent connections. Default: 64.
    #[arg(long, default_value_t = 64)]
    concurrency: usize,
}

async fn resolve_addr(host: &Option<String>, port: u16) -> Result<SocketAddr, ExitCode> {
    let host = match host {
        Some(h) => h.clone(),
        None => {
            eprintln!("Error: --host is required");
            return Err(ExitCode::from(2));
        }
    };

    if let Ok(addr) = host.parse() {
        return Ok(addr);
    }

    let endpoint = host_endpoint(&host, port);
    let mut addrs = tokio::net::lookup_host(&endpoint).await.map_err(|e| {
        eprintln!("Error: failed to resolve address '{endpoint}': {e}");
        ExitCode::from(2)
    })?;
    addrs.next().ok_or_else(|| {
        eprintln!("Error: address '{endpoint}' resolved to no socket addresses");
        ExitCode::from(2)
    })
}

fn host_endpoint(host: &str, port: u16) -> String {
    if host_has_port(host) {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
}

fn host_has_port(host: &str) -> bool {
    if host.parse::<SocketAddr>().is_ok() {
        return true;
    }
    host.rsplit_once(':')
        .is_some_and(|(_, maybe_port)| maybe_port.parse::<u16>().is_ok())
}

fn client_unit_id(unit_id: Option<u8>) -> UnitId {
    UnitId(unit_id.unwrap_or(255))
}

fn server_unit_id(unit_id: Option<u8>) -> UnitId {
    UnitId(unit_id.unwrap_or(1))
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let _log_guard = match logging::init(&logging::LogConfig {
        filter: cli.log_filter.clone(),
        format: cli.log_format,
        file: cli.log_file.clone(),
    }) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("Error: failed to initialize logging: {e}");
            return ExitCode::from(2);
        }
    };

    tracing::debug!(command = ?cli.command, "parsed CLI command");

    match cli.command {
        Commands::Read(args) => {
            let addr = match resolve_addr(&cli.host, cli.port).await {
                Ok(a) => a,
                Err(code) => return code,
            };
            let unit = client_unit_id(cli.unit_id);
            let config = ClientConfig {
                unit_id: unit,
                timeout: Duration::from_secs(cli.timeout),
                ..ClientConfig::default()
            };
            let client = match ModbusClient::connect(addr, config).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error: connection failed: {e}");
                    return ExitCode::from(2);
                }
            };
            match commands::handle_read(&client, unit, &args, cli.format).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(ClientError::Exception(_)) => ExitCode::from(1),
                Err(e) => {
                    eprintln!("Error: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Commands::Write(args) => {
            let addr = match resolve_addr(&cli.host, cli.port).await {
                Ok(a) => a,
                Err(code) => return code,
            };
            let unit = client_unit_id(cli.unit_id);
            let config = ClientConfig {
                unit_id: unit,
                timeout: Duration::from_secs(cli.timeout),
                ..ClientConfig::default()
            };
            let client = match ModbusClient::connect(addr, config).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error: connection failed: {e}");
                    return ExitCode::from(2);
                }
            };
            match commands::handle_write(&client, unit, &args, cli.format).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(ClientError::Exception(_)) => ExitCode::from(1),
                Err(e) => {
                    eprintln!("Error: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Commands::Shell => {
            let addr = match resolve_addr(&cli.host, cli.port).await {
                Ok(a) => a,
                Err(code) => return code,
            };
            let shell_config = shell::ShellConfig {
                addr,
                unit_id: client_unit_id(cli.unit_id).0,
                timeout: cli.timeout,
                format: cli.format,
            };
            match shell::run(shell_config).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("Error: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Commands::Dashboard(args) => {
            let addr = match resolve_addr(&cli.host, cli.port).await {
                Ok(a) => a,
                Err(code) => return code,
            };
            let dashboard_config = dashboard::DashboardConfig {
                addr,
                unit_id: client_unit_id(cli.unit_id).0,
                timeout: cli.timeout,
                address: args.address,
                quantity: args.quantity,
                target: args.target,
                refresh_interval: Duration::from_secs(args.refresh_secs),
            };
            match dashboard::run(dashboard_config).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("Error: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Commands::Server(args) => {
            let config = server::ServerCommandConfig::from_args(args, server_unit_id(cli.unit_id));
            match server::run(config).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("Error: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Commands::Discover(args) => {
            let discover_config = discover::DiscoverConfig {
                range: args.range,
                host: cli.host.clone(),
                port: cli.port,
                unit_id_range: args.unit_ids,
                timeout: args.discover_timeout,
                concurrency: args.concurrency,
                format: cli.format,
            };
            match discover::run(discover_config).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("Error: {e}");
                    ExitCode::from(2)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn parses_dashboard_command_args() {
        let cli = Cli::try_parse_from([
            "modbus",
            "--host",
            "127.0.0.1",
            "dashboard",
            "--target",
            "coils",
            "--address",
            "10",
            "--quantity",
            "8",
            "--refresh-secs",
            "0",
        ])
        .unwrap();

        assert_eq!(cli.host.as_deref(), Some("127.0.0.1"));
        let Commands::Dashboard(args) = cli.command else {
            panic!("expected dashboard command");
        };
        assert_eq!(args.target, dashboard::DashboardTarget::Coils);
        assert_eq!(args.address, 10);
        assert_eq!(args.quantity, 8);
        assert_eq!(args.refresh_secs, 0);
    }

    #[test]
    fn parses_server_command_defaults() {
        let cli = Cli::try_parse_from(["modbus", "server"]).unwrap();

        assert_eq!(cli.unit_id, None);
        let Commands::Server(args) = cli.command else {
            panic!("expected server command");
        };
        assert_eq!(args.listen.to_string(), "0.0.0.0:5502");
        assert_eq!(args.table_size, 65_536);
        assert!(args.holding.is_empty());
    }

    #[test]
    fn client_and_server_unit_defaults_are_distinct() {
        assert_eq!(client_unit_id(None), UnitId(255));
        assert_eq!(server_unit_id(None), UnitId(1));
        assert_eq!(client_unit_id(Some(7)), UnitId(7));
        assert_eq!(server_unit_id(Some(7)), UnitId(7));
    }

    #[test]
    fn host_endpoint_adds_port_to_bare_hostnames() {
        assert_eq!(host_endpoint("plc.local", 502), "plc.local:502");
        assert_eq!(host_endpoint("127.0.0.1", 1502), "127.0.0.1:1502");
        assert_eq!(host_endpoint("plc.local:1502", 502), "plc.local:1502");
    }

    #[test]
    fn parses_logging_args() {
        let cli = Cli::try_parse_from([
            "modbus",
            "--host",
            "127.0.0.1",
            "--log-filter",
            "rusty_modbus_client=debug",
            "--log-format",
            "json",
            "--log-file",
            "modbus.log",
            "read",
            "hr",
            "0",
            "1",
        ])
        .unwrap();

        assert_eq!(cli.log_filter.as_deref(), Some("rusty_modbus_client=debug"));
        assert_eq!(cli.log_format, logging::LogFormat::Json);
        assert_eq!(
            cli.log_file.as_deref(),
            Some(std::path::Path::new("modbus.log"))
        );
    }
}
