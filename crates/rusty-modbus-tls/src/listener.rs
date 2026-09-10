//! TLS server listener — accepts incoming Modbus/TCP Security connections.

use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::{debug, trace};

use crate::TlsServerAcceptor;
use crate::config::TlsServerConfig;
use crate::connect::{TlsRecvStream, TlsSink};
use crate::error::TlsError;

/// TLS server listener with mutual authentication.
pub struct TlsServerListener {
    tcp_listener: TcpListener,
    tls_acceptor: TlsServerAcceptor,
}

impl TlsServerListener {
    /// Bind and prepare for TLS-secured Modbus connections.
    #[tracing::instrument(level = "debug", skip(config), fields(addr = %addr))]
    pub async fn bind(addr: SocketAddr, config: &TlsServerConfig) -> Result<Self, TlsError> {
        let tls_acceptor = TlsServerAcceptor::new(config)?;

        let tcp_listener = TcpListener::bind(addr).await.map_err(TlsError::Io)?;
        debug!(addr = %tcp_listener.local_addr()?, "TLS Modbus listener bound");

        Ok(Self {
            tcp_listener,
            tls_acceptor,
        })
    }

    /// Accept the next TLS connection, returning split transport halves, the
    /// peer address, and the client's Modbus role.
    ///
    /// The role is extracted from the client's leaf certificate (Security Spec
    /// §8.4, R-21) — `Some(role)` if the certificate carries the Modbus role
    /// extension, `None` for a NULL role (R-23). Pass it into an
    /// [`AuthzRequest`](crate::AuthzRequest) and
    /// [`TlsServerConfig::authorize`](crate::TlsServerConfig::authorize) to
    /// enforce per-request authorization.
    pub async fn accept(
        &self,
    ) -> Result<(TlsSink, TlsRecvStream, SocketAddr, Option<String>), TlsError> {
        let (tcp_stream, addr) = self.tcp_listener.accept().await.map_err(TlsError::Io)?;
        trace!(peer_addr = %addr, "accepted TCP socket for TLS Modbus connection");
        tcp_stream.set_nodelay(true)?;

        debug!(peer_addr = %addr, "starting TLS server handshake");
        let (sink, stream, role) = self.tls_acceptor.accept(tcp_stream, None, None).await?;
        debug!(
            peer_addr = %addr,
            role = role.as_deref().unwrap_or("null"),
            "TLS server handshake complete"
        );

        Ok((sink, stream, addr, role))
    }

    /// Local address the listener is bound to.
    pub fn local_addr(&self) -> Result<SocketAddr, TlsError> {
        Ok(self.tcp_listener.local_addr()?)
    }
}
