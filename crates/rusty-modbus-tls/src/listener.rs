//! TLS server listener — accepts incoming Modbus/TCP Security connections.

use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::StreamExt;
use rusty_modbus_frame::mbap::MbapCodec;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_util::codec::Framed;

use crate::config::TlsServerConfig;
use crate::connect::{TlsRecvStream, TlsSink};
use crate::error::TlsError;
use crate::tls_config;

/// TLS server listener with mutual authentication.
pub struct TlsServerListener {
    tcp_listener: TcpListener,
    tls_acceptor: TlsAcceptor,
}

impl TlsServerListener {
    /// Bind and prepare for TLS-secured Modbus connections.
    pub async fn bind(
        addr: SocketAddr,
        config: &TlsServerConfig,
    ) -> Result<Self, TlsError> {
        let rustls_config = tls_config::build_server_config(config)?;
        let tls_acceptor = TlsAcceptor::from(Arc::new(rustls_config));

        let tcp_listener = TcpListener::bind(addr).await.map_err(TlsError::Io)?;

        Ok(Self {
            tcp_listener,
            tls_acceptor,
        })
    }

    /// Accept the next TLS connection, returning split transport halves.
    pub async fn accept(&self) -> Result<(TlsSink, TlsRecvStream, SocketAddr), TlsError> {
        let (tcp_stream, addr) = self.tcp_listener.accept().await.map_err(TlsError::Io)?;
        tcp_stream.set_nodelay(true)?;

        let tls_stream = self
            .tls_acceptor
            .accept(tcp_stream)
            .await
            .map_err(|e| TlsError::Handshake(e.to_string()))?;

        let framed = Framed::new(tls_stream, MbapCodec);
        let (sink, stream) = framed.split();

        Ok((
            TlsSink::new(sink, None),
            TlsRecvStream::new(stream, None),
            addr,
        ))
    }

    /// Local address the listener is bound to.
    pub fn local_addr(&self) -> Result<SocketAddr, TlsError> {
        Ok(self.tcp_listener.local_addr()?)
    }
}
