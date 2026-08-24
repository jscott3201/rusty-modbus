//! CLI Modbus/TCP server command.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use clap::Args;
use rusty_modbus::server::{
    InMemoryStore, ModbusServer, ServerConfig, ServerMetrics, ShutdownOutcome, StoreConfig,
};
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

    /// Seconds allowed for admitted requests to drain during shutdown.
    #[arg(
        long,
        default_value_t = 10.0,
        value_parser = parse_finite_positive_seconds
    )]
    pub shutdown_timeout_secs: f64,

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
    shutdown_timeout: Duration,
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
            shutdown_timeout: Duration::from_secs_f64(args.shutdown_timeout_secs),
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
            shutdown_timeout: config.shutdown_timeout,
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
    let outcome = server.stop().await;
    let metrics = server.metrics();
    let report = shutdown_report(outcome, metrics);
    eprintln!("{report}");
    tracing::info!(
        addr = %local_addr,
        %outcome,
        active_connections = metrics.active_connections,
        active_requests = metrics.active_requests,
        accepted_connections = metrics.accepted_connections,
        access_denied_connections = metrics.access_denied_connections,
        connection_limit_rejections = metrics.connection_limit_rejections,
        accept_errors = metrics.accept_errors,
        "Modbus CLI server stopped"
    );
    Ok(())
}

fn shutdown_report(outcome: ShutdownOutcome, metrics: ServerMetrics) -> String {
    format!(
        "Modbus server shutdown {outcome}: active_connections={}, active_requests={}, \
         accepted_connections={}, access_denied_connections={}, \
         connection_limit_rejections={}, accept_errors={}",
        metrics.active_connections,
        metrics.active_requests,
        metrics.accepted_connections,
        metrics.access_denied_connections,
        metrics.connection_limit_rejections,
        metrics.accept_errors,
    )
}

fn parse_finite_positive_seconds(value: &str) -> Result<f64, String> {
    let seconds = value
        .parse::<f64>()
        .map_err(|error| format!("invalid duration {value:?}: {error}"))?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(String::from("duration must be finite and positive"));
    }
    Duration::try_from_secs_f64(seconds)
        .map_err(|error| format!("duration is out of range: {error}"))?;
    Ok(seconds)
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

    #[test]
    fn shutdown_report_includes_outcome_and_all_counters() {
        let report = shutdown_report(
            ShutdownOutcome::Forced,
            ServerMetrics {
                active_connections: 0,
                active_requests: 0,
                accepted_connections: 5,
                access_denied_connections: 1,
                connection_limit_rejections: 2,
                accept_errors: 3,
            },
        );

        assert_eq!(
            report,
            "Modbus server shutdown forced: active_connections=0, active_requests=0, \
             accepted_connections=5, access_denied_connections=1, \
             connection_limit_rejections=2, accept_errors=3"
        );
    }

    #[test]
    fn shutdown_timeout_parser_rejects_non_finite_and_non_positive_values() {
        for value in ["0", "-1", "NaN", "inf", "1e300"] {
            assert!(parse_finite_positive_seconds(value).is_err(), "{value}");
        }
        assert_eq!(parse_finite_positive_seconds("2.5"), Ok(2.5));
    }
}
