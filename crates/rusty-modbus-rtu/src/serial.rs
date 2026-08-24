//! Serial (RS-232/RS-485) transport for physical Modbus RTU devices.
//!
//! Gated behind the `serial` feature. Requires `tokio-serial` for async
//! serial port access.
//! The write half enforces the Modbus RTU t3.5 silent interval before each
//! subsequent transmit. Receive-side t1.5 gap-based frame aborts are not exposed
//! by `tokio_util::codec::Framed`; malformed/gapped frames are rejected by CRC
//! once delivered by the serial driver.

use std::io;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rusty_modbus_frame::frame::Frame;
use rusty_modbus_frame::rtu::RtuCodec;
use rusty_modbus_tcp::TransportError;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use tokio::time::{Instant, sleep, timeout};
use tokio_serial::SerialPortBuilderExt;
use tokio_util::codec::Framed;

use crate::config::{DataBits, Parity, ResolvedRtuConfig, RtuConfig, StopBits, StrictRtuConfig};
use crate::error::RtuError;
use crate::unit_id::RtuUnitIdRole;

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

        Ok(split_serial_stream(
            port,
            config.response_timeout,
            config.interframe_delay(),
            None,
        ))
    }

    /// Open a serial port with a validated physical RTU configuration.
    ///
    /// One [`ResolvedRtuConfig`] snapshot supplies the native serial settings,
    /// response timeout, and t3.5 transmit delay. Both returned halves expose
    /// that snapshot through their `resolved_config` getters.
    ///
    /// # Errors
    ///
    /// Returns [`RtuError::SerialPort`] if the port cannot be opened or
    /// configured with the resolved parameters.
    pub fn open_strict(
        path: &str,
        config: &StrictRtuConfig,
    ) -> Result<(SerialSink, SerialRecvStream), RtuError> {
        let resolved = config.resolve();
        let settings = NativeSerialSettings::from(&resolved);
        let port = tokio_serial::new(path, settings.baud_rate)
            .data_bits(settings.data_bits)
            .parity(settings.parity)
            .stop_bits(settings.stop_bits)
            .open_native_async()
            .map_err(|e| RtuError::SerialPort(e.to_string()))?;

        Ok(split_serial_stream(
            port,
            resolved.response_timeout(),
            resolved.t3_5(),
            Some(resolved),
        ))
    }
}

fn split_serial_stream(
    port: tokio_serial::SerialStream,
    response_timeout: Duration,
    interframe_delay: Duration,
    resolved_config: Option<ResolvedRtuConfig>,
) -> (SerialSink, SerialRecvStream) {
    let framed = Framed::new(port, RtuCodec);
    let (sink, stream) = framed.split();

    (
        SerialSink {
            inner: sink,
            write_timeout: Some(response_timeout),
            interframe_delay,
            last_tx_end: None,
            resolved_config,
        },
        SerialRecvStream {
            inner: stream,
            read_timeout: Some(response_timeout),
            resolved_config,
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativeSerialSettings {
    baud_rate: u32,
    data_bits: tokio_serial::DataBits,
    parity: tokio_serial::Parity,
    stop_bits: tokio_serial::StopBits,
}

impl From<&ResolvedRtuConfig> for NativeSerialSettings {
    fn from(config: &ResolvedRtuConfig) -> Self {
        Self {
            baud_rate: config.baud_rate(),
            data_bits: convert_data_bits(config.data_bits()),
            parity: convert_parity(config.parity()),
            stop_bits: convert_stop_bits(config.stop_bits()),
        }
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
///
/// A half returned by [`SerialTransport::open_strict`] rejects destinations
/// above 247 before waiting for t3.5 or encoding the frame. The failure is a
/// [`TransportError::Io`] with [`io::ErrorKind::InvalidInput`] and a typed
/// [`crate::RtuUnitIdError`] source.
pub struct SerialSink {
    inner: InnerSink,
    write_timeout: Option<Duration>,
    interframe_delay: Duration,
    last_tx_end: Option<Instant>,
    resolved_config: Option<ResolvedRtuConfig>,
}

impl SerialSink {
    /// Return the settings snapshot used by strict open, or `None` for legacy open.
    #[must_use]
    pub const fn resolved_config(&self) -> Option<&ResolvedRtuConfig> {
        self.resolved_config.as_ref()
    }
}

impl TransportSink for SerialSink {
    async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        if self.resolved_config.is_some() {
            validate_strict_frame_unit_id(&frame, RtuUnitIdRole::ClientDestination)?;
        }
        wait_for_interframe_silence(self.last_tx_end, self.interframe_delay).await;

        let fut = SinkExt::send(&mut self.inner, frame);
        let result = if let Some(dur) = self.write_timeout {
            timeout(dur, fut)
                .await
                .map_err(|_| TransportError::Timeout)?
                .map_err(TransportError::Frame)
        } else {
            fut.await.map_err(TransportError::Frame)
        };

        if result.is_ok() {
            self.last_tx_end = Some(Instant::now());
        }
        result
    }
}

/// Read half of a serial transport.
///
/// A half returned by [`SerialTransport::open_strict`] rejects decoded source
/// identifiers outside 1 through 247. The failure is a [`TransportError::Io`]
/// with [`io::ErrorKind::InvalidData`] and a typed [`crate::RtuUnitIdError`]
/// source.
pub struct SerialRecvStream {
    inner: InnerStream,
    read_timeout: Option<Duration>,
    resolved_config: Option<ResolvedRtuConfig>,
}

impl SerialRecvStream {
    /// Return the settings snapshot used by strict open, or `None` for legacy open.
    #[must_use]
    pub const fn resolved_config(&self) -> Option<&ResolvedRtuConfig> {
        self.resolved_config.as_ref()
    }
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
            Some(Ok(frame)) => {
                if self.resolved_config.is_some() {
                    validate_strict_frame_unit_id(&frame, RtuUnitIdRole::ResponderSource)?;
                }
                Ok(frame)
            }
            Some(Err(e)) => Err(TransportError::Frame(e)),
            None => Err(TransportError::Disconnected),
        }
    }
}

fn validate_strict_frame_unit_id(frame: &Frame, role: RtuUnitIdRole) -> Result<(), TransportError> {
    role.validate(frame.unit_id()).map_err(|error| {
        let kind = match role {
            RtuUnitIdRole::ClientDestination => io::ErrorKind::InvalidInput,
            RtuUnitIdRole::ResponderSource => io::ErrorKind::InvalidData,
        };
        TransportError::Io(io::Error::new(kind, error))
    })
}

fn remaining_interframe_delay(
    last_tx_end: Option<Instant>,
    now: Instant,
    interframe_delay: Duration,
) -> Duration {
    let Some(last_tx_end) = last_tx_end else {
        return Duration::ZERO;
    };
    interframe_delay.saturating_sub(now.saturating_duration_since(last_tx_end))
}

async fn wait_for_interframe_silence(last_tx_end: Option<Instant>, interframe_delay: Duration) {
    let remaining = remaining_interframe_delay(last_tx_end, Instant::now(), interframe_delay);
    if !remaining.is_zero() {
        sleep(remaining).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusty_modbus_frame::frame::FrameHeader;

    use crate::config::{RtuSerialFormat, RtuTimingMode};
    use crate::unit_id::RtuUnitIdError;

    fn frame(unit_id: u8) -> Frame {
        Frame {
            header: FrameHeader::Rtu { unit_id },
            pdu: Bytes::new(),
        }
    }

    fn assert_unit_id_io_error(
        error: TransportError,
        expected_kind: io::ErrorKind,
        expected_unit_id: u8,
        expected_role: RtuUnitIdRole,
    ) {
        let TransportError::Io(error) = error else {
            panic!("expected I/O error, got {error:?}");
        };
        assert_eq!(error.kind(), expected_kind);
        let source = error
            .get_ref()
            .and_then(|source| source.downcast_ref::<RtuUnitIdError>())
            .expect("typed RTU Unit ID error must remain the I/O error source");
        assert_eq!(source.unit_id(), expected_unit_id);
        assert_eq!(source.role(), expected_role);
    }

    #[test]
    fn first_serial_write_needs_no_interframe_delay() {
        let now = Instant::now();

        assert_eq!(
            remaining_interframe_delay(None, now, Duration::from_millis(4)),
            Duration::ZERO
        );
    }

    #[test]
    fn serial_write_waits_remaining_interframe_delay() {
        let now = Instant::now();
        let last = now - Duration::from_millis(1);

        assert_eq!(
            remaining_interframe_delay(Some(last), now, Duration::from_millis(4)),
            Duration::from_millis(3)
        );
    }

    #[test]
    fn serial_write_needs_no_delay_after_full_silence() {
        let now = Instant::now();
        let last = now - Duration::from_millis(10);

        assert_eq!(
            remaining_interframe_delay(Some(last), now, Duration::from_millis(4)),
            Duration::ZERO
        );
    }

    #[test]
    fn strict_serial_settings_map_every_valid_format() {
        let cases = [
            (
                RtuSerialFormat::EightEvenOne,
                tokio_serial::Parity::Even,
                tokio_serial::StopBits::One,
            ),
            (
                RtuSerialFormat::EightOddOne,
                tokio_serial::Parity::Odd,
                tokio_serial::StopBits::One,
            ),
            (
                RtuSerialFormat::EightNoneTwo,
                tokio_serial::Parity::None,
                tokio_serial::StopBits::Two,
            ),
        ];

        for (format, parity, stop_bits) in cases {
            let resolved = StrictRtuConfig::new(57_600, format, Duration::from_millis(400))
                .unwrap()
                .resolve();
            let native = NativeSerialSettings::from(&resolved);

            assert_eq!(native.baud_rate, resolved.baud_rate());
            assert_eq!(native.data_bits, tokio_serial::DataBits::Eight);
            assert_eq!(native.parity, parity);
            assert_eq!(native.stop_bits, stop_bits);
            assert_eq!(resolved.response_timeout(), Duration::from_millis(400));
            assert_eq!(resolved.t3_5(), Duration::from_micros(1_750));
            assert_eq!(
                resolved.timing_mode(),
                RtuTimingMode::FixedHighSpeedRecommendation
            );
        }
    }

    #[test]
    fn strict_send_unit_validation_precedes_encoding() {
        validate_strict_frame_unit_id(&frame(0), RtuUnitIdRole::ClientDestination).unwrap();

        for unit_id in [248, 255] {
            let error =
                validate_strict_frame_unit_id(&frame(unit_id), RtuUnitIdRole::ClientDestination)
                    .unwrap_err();
            assert_unit_id_io_error(
                error,
                io::ErrorKind::InvalidInput,
                unit_id,
                RtuUnitIdRole::ClientDestination,
            );
        }
    }

    #[test]
    fn strict_receive_rejects_broadcast_and_reserved_sources_after_decode() {
        for unit_id in [0, 248, 255] {
            let error =
                validate_strict_frame_unit_id(&frame(unit_id), RtuUnitIdRole::ResponderSource)
                    .unwrap_err();
            assert_unit_id_io_error(
                error,
                io::ErrorKind::InvalidData,
                unit_id,
                RtuUnitIdRole::ResponderSource,
            );
        }
        for unit_id in [1, 247] {
            validate_strict_frame_unit_id(&frame(unit_id), RtuUnitIdRole::ResponderSource).unwrap();
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn strict_halves_expose_one_snapshot_and_reject_before_delay() {
        let resolved = StrictRtuConfig::default().resolve();
        let (port, _peer) = tokio_serial::SerialStream::pair().unwrap();
        let (mut sink, stream) = split_serial_stream(
            port,
            resolved.response_timeout(),
            Duration::from_secs(10),
            Some(resolved),
        );

        assert_eq!(sink.resolved_config(), Some(&resolved));
        assert_eq!(stream.resolved_config(), Some(&resolved));

        sink.last_tx_end = Some(Instant::now());
        let error = timeout(Duration::from_millis(50), sink.send(frame(248)))
            .await
            .expect("Unit ID validation must run before the ten-second delay")
            .unwrap_err();
        assert_unit_id_io_error(
            error,
            io::ErrorKind::InvalidInput,
            248,
            RtuUnitIdRole::ClientDestination,
        );
    }
}
