//! TLS transport configuration.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rusty_modbus_types::{Address, FunctionCode, Quantity, UnitId};

/// TLS client connection configuration.
#[derive(Debug, Clone)]
pub struct TlsClientConfig {
    /// CA certificate for server verification.
    pub ca_cert: PathBuf,
    /// Client domain certificate (R-19: mutual auth).
    pub client_cert: PathBuf,
    /// Client private key.
    pub client_key: PathBuf,
    /// Connection timeout. Default: 5s.
    pub connect_timeout: Duration,
    /// Read timeout. Default: 30s.
    pub read_timeout: Option<Duration>,
    /// Write timeout. Default: 30s.
    pub write_timeout: Option<Duration>,
}

impl Default for TlsClientConfig {
    fn default() -> Self {
        Self {
            ca_cert: PathBuf::new(),
            client_cert: PathBuf::new(),
            client_key: PathBuf::new(),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Some(Duration::from_secs(30)),
            write_timeout: Some(Duration::from_secs(30)),
        }
    }
}

/// TLS server listener configuration.
#[derive(Clone)]
pub struct TlsServerConfig {
    /// Server certificate.
    pub server_cert: PathBuf,
    /// Server private key.
    pub server_key: PathBuf,
    /// CA certificate for client certificate verification.
    pub ca_cert: PathBuf,
    /// Require client certificate. Default: `true` (R-06: mutual auth MUST).
    pub require_client_cert: bool,
    /// Per-request authorization callback.
    pub authz_callback: Option<AuthzCallback>,
    /// Maximum connections. Default: 64.
    pub max_connections: usize,
}

impl std::fmt::Debug for TlsServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsServerConfig")
            .field("server_cert", &self.server_cert)
            .field("require_client_cert", &self.require_client_cert)
            .field("has_authz", &self.authz_callback.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for TlsServerConfig {
    fn default() -> Self {
        Self {
            server_cert: PathBuf::new(),
            server_key: PathBuf::new(),
            ca_cert: PathBuf::new(),
            require_client_cert: true,
            authz_callback: None,
            max_connections: 64,
        }
    }
}

/// Authorization callback type — invoked on every request.
pub type AuthzCallback = Arc<dyn Fn(&AuthzRequest) -> AuthzDecision + Send + Sync>;

/// Authorization request context passed to the callback.
#[derive(Debug, Clone)]
pub struct AuthzRequest {
    /// Client role extracted from certificate (None if no role extension).
    pub role: Option<String>,
    /// The Modbus function code being requested.
    pub function_code: FunctionCode,
    /// The target unit ID.
    pub unit_id: UnitId,
    /// Target address (if applicable to the function code).
    pub address: Option<Address>,
    /// Quantity (if applicable).
    pub quantity: Option<Quantity>,
}

/// Authorization decision returned by the callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthzDecision {
    /// Request is authorized — proceed normally.
    Authorized,
    /// Request is denied — server returns `IllegalFunction` (0x01) per R-31.
    NotAuthorized,
}
