//! TLS client and server transport halves.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use rustls::pki_types::ServerName;
use rusty_modbus_frame::error::FrameError;
use rusty_modbus_frame::frame::Frame;
use rusty_modbus_frame::mbap::MbapCodec;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_util::codec::Framed;
use tracing::{debug, trace, warn};

use crate::config::TlsClientConfig;
use crate::error::TlsError;
use crate::tls_config;

/// TLS client transport — connects to a Modbus/TCP Security server.
pub struct TlsTransport;

impl TlsTransport {
    /// Connect to a Modbus/TCP Security server with mutual TLS authentication.
    ///
    /// Enforces TLS 1.3 with mutual x.509v3 authentication.
    ///
    /// # Errors
    ///
    /// Returns [`TlsError`] on handshake failure, certificate issues, or I/O errors.
    #[tracing::instrument(level = "debug", skip(config), fields(addr = %addr, server_name = ?config.server_name))]
    pub async fn connect(
        addr: SocketAddr,
        config: &TlsClientConfig,
    ) -> Result<(TlsSink, TlsRecvStream), TlsError> {
        debug!("building TLS client configuration");
        let rustls_config = tls_config::build_client_config(config)?;
        let connector = TlsConnector::from(Arc::new(rustls_config));

        debug!(connect_timeout = ?config.connect_timeout, "connecting TLS TCP socket");
        let tcp_stream = timeout(config.connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| TlsError::Timeout)?
            .map_err(TlsError::Io)?;

        tcp_stream.set_nodelay(true)?;

        // Verify the server certificate against the configured hostname (SNI +
        // DNS-SAN check) when set, otherwise against the connection IP address.
        let server_name = match &config.server_name {
            Some(name) => ServerName::try_from(name.clone())
                .map_err(|_| TlsError::Certificate(format!("invalid server name: {name}")))?,
            None => ServerName::IpAddress(addr.ip().into()),
        };
        debug!("starting TLS handshake");
        let tls_stream = connector
            .connect(server_name, tcp_stream)
            .await
            .map_err(|e| TlsError::Handshake(e.to_string()))?;
        debug!("TLS client handshake complete");

        let framed = Framed::new(tls_stream, MbapCodec);
        let (sink, stream) = framed.split();

        Ok((
            TlsSink::new(sink, config.write_timeout),
            TlsRecvStream::new(stream, config.read_timeout),
        ))
    }
}

/// Write half of a TLS transport (works for both client and server sides).
pub struct TlsSink {
    inner: Box<dyn Sink<Frame, Error = FrameError> + Send + Unpin>,
    write_timeout: Option<Duration>,
}

impl TlsSink {
    /// Create from any `Sink<Frame>` implementor.
    pub(crate) fn new<S>(sink: S, write_timeout: Option<Duration>) -> Self
    where
        S: Sink<Frame, Error = FrameError> + Send + Unpin + 'static,
    {
        Self {
            inner: Box::new(sink),
            write_timeout,
        }
    }
}

impl rusty_modbus_tcp::TransportSink for TlsSink {
    async fn send(&mut self, frame: Frame) -> Result<(), rusty_modbus_tcp::TransportError> {
        let unit_id = frame.unit_id();
        let pdu_len = frame.pdu.len();
        trace!(unit_id, pdu_len, "sending TLS Modbus frame");
        let fut = SinkExt::send(&mut self.inner, frame);
        let result = if let Some(dur) = self.write_timeout {
            match timeout(dur, fut).await {
                Ok(result) => result.map_err(rusty_modbus_tcp::TransportError::Frame),
                Err(_) => Err(rusty_modbus_tcp::TransportError::Timeout),
            }
        } else {
            fut.await.map_err(rusty_modbus_tcp::TransportError::Frame)
        };
        if let Err(error) = &result {
            warn!(unit_id, pdu_len, error = %error, "failed to send TLS Modbus frame");
        }
        result
    }
}

/// Read half of a TLS transport (works for both client and server sides).
pub struct TlsRecvStream {
    inner: Box<dyn Stream<Item = Result<Frame, FrameError>> + Send + Unpin>,
    read_timeout: Option<Duration>,
}

impl TlsRecvStream {
    /// Create from any `Stream<Item = Result<Frame, FrameError>>` implementor.
    pub(crate) fn new<S>(stream: S, read_timeout: Option<Duration>) -> Self
    where
        S: Stream<Item = Result<Frame, FrameError>> + Send + Unpin + 'static,
    {
        Self {
            inner: Box::new(stream),
            read_timeout,
        }
    }
}

impl rusty_modbus_tcp::TransportStream for TlsRecvStream {
    async fn recv(&mut self) -> Result<Frame, rusty_modbus_tcp::TransportError> {
        let fut = self.inner.next();
        let item = if let Some(dur) = self.read_timeout {
            if let Ok(item) = timeout(dur, fut).await {
                item
            } else {
                trace!(timeout = ?dur, "timed out waiting for TLS Modbus frame");
                return Err(rusty_modbus_tcp::TransportError::Timeout);
            }
        } else {
            fut.await
        };

        match item {
            Some(Ok(frame)) => {
                trace!(
                    unit_id = frame.unit_id(),
                    pdu_len = frame.pdu.len(),
                    "received TLS Modbus frame"
                );
                Ok(frame)
            }
            Some(Err(e)) => {
                warn!(error = %e, "failed to decode TLS Modbus frame");
                Err(rusty_modbus_tcp::TransportError::Frame(e))
            }
            None => {
                debug!("TLS Modbus stream disconnected");
                Err(rusty_modbus_tcp::TransportError::Disconnected)
            }
        }
    }
}
