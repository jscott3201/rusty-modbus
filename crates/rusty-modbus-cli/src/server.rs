//! CLI Modbus/TCP server command.

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Args;
use rusty_modbus::server::{InMemoryStore, ModbusServer, ServerConfig, StoreConfig};
use rusty_modbus_types::UnitId;

/// Arguments for the `server` subcommand.
#[derive(Args, Debug)]
pub struct ServerArgs {
    /// Address to listen on. Uses a non-privileged port for non-root containers.
    #[arg(long, default_value = "0.0.0.0:5502")]
    pub listen: SocketAddr,

    /// Maximum concurrent TCP connections.
    #[arg(long, default_value_t = 64)]
    pub max_connections: usize,

    /// Maximum concurrent transactions per connection.
    #[arg(long, default_value_t = 16)]
    pub max_transactions: u16,

    /// Size for each in-memory Modbus data table.
    #[arg(long, default_value_t = 65_536)]
    pub table_size: usize,

    /// Seed holding register values as ADDR=VALUE. Values may be decimal or 0x-prefixed.
    #[arg(long = "holding", value_name = "ADDR=VALUE")]
    pub holding: Vec<String>,

    /// Seed input register values as ADDR=VALUE. Values may be decimal or 0x-prefixed.
    #[arg(long = "input", value_name = "ADDR=VALUE")]
    pub input: Vec<String>,

    /// Seed coil values as ADDR=on/off.
    #[arg(long = "coil", value_name = "ADDR=on|off")]
    pub coil: Vec<String>,

    /// Seed discrete input values as ADDR=on/off.
    #[arg(long = "discrete", value_name = "ADDR=on|off")]
    pub discrete: Vec<String>,
}

/// Fully resolved server command configuration.
#[derive(Debug)]
pub struct ServerCommandConfig {
    listen: SocketAddr,
    unit_id: UnitId,
    max_connections: usize,
    max_transactions: u16,
    table_size: usize,
    holding: Vec<String>,
    input: Vec<String>,
    coil: Vec<String>,
    discrete: Vec<String>,
}

impl ServerCommandConfig {
    /// Build a resolved configuration from CLI arguments and the global unit ID.
    pub fn from_args(args: ServerArgs, unit_id: UnitId) -> Self {
        Self {
            listen: args.listen,
            unit_id,
            max_connections: args.max_connections,
            max_transactions: args.max_transactions,
            table_size: args.table_size,
            holding: args.holding,
            input: args.input,
            coil: args.coil,
            discrete: args.discrete,
        }
    }
}

/// Run an in-memory Modbus/TCP server until SIGINT/SIGTERM.
pub async fn run(config: ServerCommandConfig) -> Result<(), String> {
    let store = Arc::new(
        InMemoryStore::try_new(StoreConfig {
            coil_count: config.table_size,
            discrete_input_count: config.table_size,
            holding_register_count: config.table_size,
            input_register_count: config.table_size,
        })
        .map_err(|error| format!("invalid store configuration: {error}"))?,
    );

    seed_registers(
        &store,
        "holding",
        &config.holding,
        |store, address, value| store.set_holding_register(address, value),
    )?;
    seed_registers(&store, "input", &config.input, |store, address, value| {
        store.set_input_register(address, value)
    })?;
    seed_bits(&store, "coil", &config.coil, |store, address, value| {
        store.set_coil(address, value)
    })?;
    seed_bits(
        &store,
        "discrete",
        &config.discrete,
        |store, address, value| store.set_discrete_input(address, value),
    )?;

    let server = ModbusServer::start(
        ServerConfig {
            listen_addr: config.listen,
            unit_id: config.unit_id,
            max_connections: config.max_connections,
            max_transactions: config.max_transactions,
            ..ServerConfig::default()
        },
        Arc::clone(&store),
    )
    .await
    .map_err(|error| format!("failed to start server: {error}"))?;

    let local_addr = server.local_addr();
    eprintln!(
        "Modbus server listening on {local_addr} (unit {})",
        config.unit_id.0
    );
    tracing::info!(
        addr = %local_addr,
        unit_id = config.unit_id.0,
        "Modbus CLI server listening"
    );

    wait_for_shutdown_signal().await?;
    tracing::info!(addr = %local_addr, "Modbus CLI server shutting down");
    server.stop().await;
    Ok(())
}

fn seed_registers(
    store: &InMemoryStore,
    kind: &'static str,
    values: &[String],
    set: impl Fn(&InMemoryStore, u16, u16) -> Result<(), rusty_modbus::server::StoreError>,
) -> Result<(), String> {
    for raw in values {
        let (address, value) = parse_assignment(raw, parse_u16)?;
        set(store, address, value)
            .map_err(|error| format!("invalid {kind} seed {raw:?}: {error}"))?;
    }
    Ok(())
}

fn seed_bits(
    store: &InMemoryStore,
    kind: &'static str,
    values: &[String],
    set: impl Fn(&InMemoryStore, u16, bool) -> Result<(), rusty_modbus::server::StoreError>,
) -> Result<(), String> {
    for raw in values {
        let (address, value) = parse_assignment(raw, parse_bool)?;
        set(store, address, value)
            .map_err(|error| format!("invalid {kind} seed {raw:?}: {error}"))?;
    }
    Ok(())
}

fn parse_assignment<T>(
    raw: &str,
    parse_value: impl FnOnce(&str) -> Result<T, String>,
) -> Result<(u16, T), String> {
    let Some((address, value)) = raw.split_once('=') else {
        return Err(format!("invalid seed {raw:?}; expected ADDR=VALUE"));
    };
    Ok((parse_u16(address)?, parse_value(value)?))
}

fn parse_u16(value: &str) -> Result<u16, String> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16)
            .map_err(|error| format!("invalid u16 value {value:?}: {error}"))
    } else {
        value
            .parse()
            .map_err(|error| format!("invalid u16 value {value:?}: {error}"))
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "on" | "true" | "1" => Ok(true),
        "off" | "false" | "0" => Ok(false),
        _ => Err(format!(
            "invalid boolean value {value:?}; expected on/off, true/false, or 1/0"
        )),
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<(), String> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|error| format!("failed to install SIGTERM handler: {error}"))?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            result.map_err(|error| format!("failed to wait for SIGINT: {error}"))
        }
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> Result<(), String> {
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| format!("failed to wait for shutdown signal: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_seed_assignment_accepts_decimal_and_hex() {
        assert_eq!(
            parse_assignment("10=0xBEEF", parse_u16).unwrap(),
            (10, 0xBEEF)
        );
        assert_eq!(parse_assignment("0x10=42", parse_u16).unwrap(), (16, 42));
    }

    #[test]
    fn parse_seed_assignment_rejects_missing_separator() {
        assert!(parse_assignment("10:42", parse_u16).is_err());
    }

    #[test]
    fn parse_bool_accepts_common_forms_case_insensitively() {
        assert!(parse_bool("ON").unwrap());
        assert!(parse_bool("true").unwrap());
        assert!(parse_bool("1").unwrap());
        assert!(!parse_bool("off").unwrap());
        assert!(!parse_bool("False").unwrap());
        assert!(!parse_bool("0").unwrap());
    }
}
