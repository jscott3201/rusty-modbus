//! CLI tracing subscriber setup.

use std::path::PathBuf;

use clap::ValueEnum;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_LOG_FILTER: &str = "warn";

/// Log output format for diagnostic events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum LogFormat {
    /// Compact human-readable logs.
    Human,
    /// Newline-delimited JSON logs.
    Json,
}

/// Logging configuration parsed from CLI flags.
#[derive(Debug, Clone)]
pub(crate) struct LogConfig {
    pub(crate) filter: Option<String>,
    pub(crate) format: LogFormat,
    pub(crate) file: Option<PathBuf>,
}

impl LogConfig {
    pub(crate) fn filter(&self) -> Result<EnvFilter, tracing_subscriber::filter::ParseError> {
        match &self.filter {
            Some(filter) => EnvFilter::try_new(filter),
            None => EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new(DEFAULT_LOG_FILTER)),
        }
    }
}

/// Keeps the non-blocking writer alive until process exit.
pub(crate) struct LogGuard {
    _guard: WorkerGuard,
}

/// Initialize process-wide tracing output.
pub(crate) fn init(
    config: &LogConfig,
) -> Result<LogGuard, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let filter = config.filter()?;
    let (writer, guard) = match &config.file {
        Some(path) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            tracing_appender::non_blocking(file)
        }
        None => tracing_appender::non_blocking(std::io::stderr()),
    };

    match config.format {
        LogFormat::Human => tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .compact()
                    .with_ansi(config.file.is_none())
                    .with_writer(writer),
            )
            .try_init()?,
        LogFormat::Json => tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_current_span(true)
                    .with_span_list(true)
                    .with_ansi(false)
                    .with_writer(writer),
            )
            .try_init()?,
    }

    Ok(LogGuard { _guard: guard })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_filter_takes_precedence() {
        let config = LogConfig {
            filter: Some(String::from("rusty_modbus_client=debug")),
            format: LogFormat::Human,
            file: None,
        };

        assert!(config.filter().is_ok());
    }

    #[test]
    fn invalid_filter_is_rejected() {
        let config = LogConfig {
            filter: Some(String::from("rusty_modbus_client=definitely-not-a-level")),
            format: LogFormat::Human,
            file: None,
        };

        assert!(config.filter().is_err());
    }
}
