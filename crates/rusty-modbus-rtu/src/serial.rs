//! Serial (RS-232/RS-485) transport for physical Modbus RTU devices.
//!
//! Gated behind the `serial` feature. Requires `tokio-serial` for async
//! serial port access.

use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rusty_modbus_frame::frame::Frame;
use rusty_modbus_frame::rtu::RtuCodec;
use rusty_modbus_tcp::TransportError;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use tokio::time::timeout;
use tokio_serial::SerialPortBuilderExt;
use tokio_util::codec::Framed;

use crate::config::{DataBits, Parity, RtuConfig, StopBits};
use crate::error::RtuError;

type InnerSink = SplitSink<Framed<tokio_serial::SerialStream, RtuCodec>, Frame>;
type InnerStream = SplitStream<Framed<tokio_serial::SerialStream, RtuCodec>>;

/// Serial transport factory for physical Modbus RTU ports.
pub struct SerialTransport;

impl SerialTransport {
    /// Open a serial port and return split transport halves.
    ///
    /// # Errors
    ///
    /// Returns [`RtuError::SerialPort`] if the port cannot be opened or
    /// configured with the given parameters.
    pub fn open(
        path: &str,
        config: &RtuConfig,
    ) -> Result<(SerialSink, SerialRecvStream), RtuError> {
        let port = tokio_serial::new(path, config.baud_rate)
            .data_bits(convert_data_bits(config.data_bits))
            .parity(convert_parity(config.parity))
            .stop_bits(convert_stop_bits(config.stop_bits))
            .open_native_async()
            .map_err(|e| RtuError::SerialPort(e.to_string()))?;

        let framed = Framed::new(port, RtuCodec);
        let (sink, stream) = framed.split();

        let response_timeout = Some(config.response_timeout);

        Ok((
            SerialSink {
                inner: sink,
                write_timeout: response_timeout,
            },
            SerialRecvStream {
                inner: stream,
                read_timeout: response_timeout,
            },
        ))
    }
}

// ---------------------------------------------------------------------------
// Type conversions from our enums to tokio-serial enums
// ---------------------------------------------------------------------------

/// Convert [`DataBits`] to the `tokio_serial` equivalent.
#[must_use]
const fn convert_data_bits(bits: DataBits) -> tokio_serial::DataBits {
    match bits {
        DataBits::Five => tokio_serial::DataBits::Five,
        DataBits::Six => tokio_serial::DataBits::Six,
        DataBits::Seven => tokio_serial::DataBits::Seven,
        DataBits::Eight => tokio_serial::DataBits::Eight,
    }
}

/// Convert [`Parity`] to the `tokio_serial` equivalent.
#[must_use]
const fn convert_parity(parity: Parity) -> tokio_serial::Parity {
    match parity {
        Parity::None => tokio_serial::Parity::None,
        Parity::Even => tokio_serial::Parity::Even,
        Parity::Odd => tokio_serial::Parity::Odd,
    }
}

/// Convert [`StopBits`] to the `tokio_serial` equivalent.
#[must_use]
const fn convert_stop_bits(stop: StopBits) -> tokio_serial::StopBits {
    match stop {
        StopBits::One => tokio_serial::StopBits::One,
        StopBits::Two => tokio_serial::StopBits::Two,
    }
}

// ---------------------------------------------------------------------------
// Split halves
// ---------------------------------------------------------------------------

/// Write half of a serial transport.
pub struct SerialSink {
    inner: InnerSink,
    write_timeout: Option<Duration>,
}

impl TransportSink for SerialSink {
    async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        let fut = SinkExt::send(&mut self.inner, frame);
        if let Some(dur) = self.write_timeout {
            timeout(dur, fut)
                .await
                .map_err(|_| TransportError::Timeout)?
                .map_err(TransportError::Frame)
        } else {
            fut.await.map_err(TransportError::Frame)
        }
    }
}

/// Read half of a serial transport.
pub struct SerialRecvStream {
    inner: InnerStream,
    read_timeout: Option<Duration>,
}

impl TransportStream for SerialRecvStream {
    async fn recv(&mut self) -> Result<Frame, TransportError> {
        let fut = self.inner.next();
        let item = if let Some(dur) = self.read_timeout {
            timeout(dur, fut)
                .await
                .map_err(|_| TransportError::Timeout)?
        } else {
            fut.await
        };

        match item {
            Some(Ok(frame)) => Ok(frame),
            Some(Err(e)) => Err(TransportError::Frame(e)),
            None => Err(TransportError::Disconnected),
        }
    }
}
