//! TLS upgrade of caller-owned TCP sockets, separate from admission/lifecycle policy.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use rusty_modbus_frame::mbap::MbapCodec;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;
use tokio_util::codec::Framed;

use crate::{TlsError, TlsRecvStream, TlsServerConfig, TlsSink, tls_config};

/// Cloneable TLS 1.3 server upgrade using the existing certificate/fragment policy.
///
/// This low-level adapter does not enforce admission limits, a handshake timeout,
/// or authorization callbacks. Its caller owns those policies and socket guards.
#[derive(Clone)]
pub struct TlsServerAcceptor {
    inner: TlsAcceptor,
}

impl TlsServerAcceptor {
    /// Load certificates and build the existing TLS server configuration.
    pub fn new(config: &TlsServerConfig) -> Result<Self, TlsError> {
        Ok(Self {
            inner: TlsAcceptor::from(Arc::new(tls_config::build_server_config(config)?)),
        })
    }

    /// Consume an already admitted TCP socket and upgrade it to TLS MBAP halves.
    ///
    /// Socket options are preserved. Dropping this future cancels the handshake
    /// and closes its owned socket. Read/write timeouts apply to framed TLS I/O,
    /// not to the handshake. The optional owned role uses the existing permissive
    /// extractor; it is metadata, not authorization or strict R-22 validation.
    pub async fn accept(
        &self,
        socket: TcpStream,
        read_timeout: Option<Duration>,
        write_timeout: Option<Duration>,
    ) -> Result<(TlsSink, TlsRecvStream, Option<String>), TlsError> {
        let stream = self
            .inner
            .accept(socket)
            .await
            .map_err(|error| TlsError::Handshake(error.to_string()))?;
        let role = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(<[_]>::first)
            .and_then(|cert| crate::role::extract_role(cert.as_ref()));
        let (sink, stream) = server_halves(stream, read_timeout, write_timeout);
        Ok((sink, stream, role))
    }
}

fn server_halves<T: AsyncRead + AsyncWrite + Send + Unpin + 'static>(
    stream: T,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
) -> (TlsSink, TlsRecvStream) {
    let (sink, stream) = Framed::new(stream, MbapCodec).split();
    (
        TlsSink::new(sink, write_timeout),
        TlsRecvStream::new(stream, read_timeout),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_modbus_frame::frame::{Frame, FrameHeader};
    use rusty_modbus_tcp::{TransportError, TransportSink, TransportStream};
    use rusty_modbus_types::MbapHeader;

    #[tokio::test]
    async fn server_halves_enforce_read_and_blocked_write_timeouts() {
        // Deterministic post-handshake I/O seam; this is not a TLS crypto test.
        let (io, _peer) = tokio::io::duplex(1);
        let (mut sink, mut stream) = server_halves(
            io,
            Some(Duration::from_millis(1)),
            Some(Duration::from_millis(1)),
        );
        assert!(matches!(stream.recv().await, Err(TransportError::Timeout)));
        let frame = Frame {
            header: FrameHeader::Mbap(MbapHeader::new(1, 1, 5)),
            pdu: bytes::Bytes::from_static(&[3, 0, 0, 0, 1]),
        };
        assert!(matches!(
            sink.send(frame).await,
            Err(TransportError::Timeout)
        ));
    }
}
