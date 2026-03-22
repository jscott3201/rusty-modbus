//! Modbus CLI tool — read/write registers and coils from the command line.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use modbus_client::{ClientConfig, ClientError, ModbusClient};
use modbus_types::UnitId;

mod commands;
mod discover;
mod output;
mod shell;
mod shell_parser;

/// Modbus command-line diagnostic tool.
#[derive(Parser, Debug)]
#[command(name = "modbus", version, about = "Modbus CLI diagnostic tool")]
struct Cli {
    /// Target host (IP:port or just IP — port defaults to 502).
    #[arg(long, short = 'H', global = true)]
    host: Option<String>,

    /// Target port. Default: 502.
    #[arg(long, short = 'p', global = true, default_value_t = 502)]
    port: u16,

    /// Modbus unit/slave ID. Default: 255 (TCP direct device).
    #[arg(long, short = 'u', global = true, default_value_t = 255)]
    unit_id: u8,

    /// Request timeout in seconds. Default: 5.
    #[arg(long, short = 't', global = true, default_value_t = 5)]
    timeout: u64,

    /// Output format.
    #[arg(long, global = true, default_value = "human")]
    format: output::OutputFormat,

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

fn resolve_addr(host: &Option<String>, port: u16) -> Result<SocketAddr, ExitCode> {
    let host = match host {
        Some(h) => h.clone(),
        None => {
            eprintln!("Error: --host is required");
            return Err(ExitCode::from(2));
        }
    };

    let addr_str = if host.contains(':') {
        host
    } else {
        format!("{host}:{port}")
    };

    addr_str.parse().map_err(|e| {
        eprintln!("Error: invalid address '{addr_str}': {e}");
        ExitCode::from(2)
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::Read(args) => {
            let addr = match resolve_addr(&cli.host, cli.port) {
                Ok(a) => a,
                Err(code) => return code,
            };
            let config = ClientConfig {
                unit_id: UnitId(cli.unit_id),
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
            let unit = UnitId(cli.unit_id);
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
            let addr = match resolve_addr(&cli.host, cli.port) {
                Ok(a) => a,
                Err(code) => return code,
            };
            let config = ClientConfig {
                unit_id: UnitId(cli.unit_id),
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
            let unit = UnitId(cli.unit_id);
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
            let addr = match resolve_addr(&cli.host, cli.port) {
                Ok(a) => a,
                Err(code) => return code,
            };
            let shell_config = shell::ShellConfig {
                addr,
                unit_id: cli.unit_id,
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
