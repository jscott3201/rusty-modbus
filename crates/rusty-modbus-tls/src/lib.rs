//! Modbus/TCP Security transport — TLS 1.3 with mutual x.509v3 authentication.
//!
//! Implements the Modbus/TCP Security Protocol V36. After the TLS handshake,
//! MBAP framing is identical to plain TCP — the TLS layer is transparent.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

pub mod config;
pub mod connect;
pub mod error;
pub mod listener;
pub mod role;
pub(crate) mod tls_config;

pub use config::{AuthzCallback, AuthzDecision, AuthzRequest, TlsClientConfig, TlsServerConfig};
pub use connect::{TlsRecvStream, TlsSink, TlsTransport};
pub use error::TlsError;
pub use listener::TlsServerListener;
pub use role::extract_role;
