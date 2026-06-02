//! Simulator error types.

use rusty_modbus_server::ServerError;

/// Errors that can occur during simulator operations.
#[derive(Debug, thiserror::Error)]
pub enum SimError {
    /// YAML configuration parsing error.
    #[error("config parse error: {0}")]
    ConfigParse(#[from] serde_yaml_ng::Error),

    /// Invalid configuration value.
    #[error("config error: {0}")]
    Config(String),

    /// Underlying server error.
    #[error("server error: {0}")]
    Server(#[from] ServerError),
}
