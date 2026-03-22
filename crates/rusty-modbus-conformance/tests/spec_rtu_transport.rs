//! RTU Transport conformance tests.
//!
//! Verifies RTU configuration and timing against the Modbus serial
//! line specification.

use std::time::Duration;

use rusty_modbus_rtu::config::{DataBits, Parity, RtuConfig, StopBits};

// ── RTU Config Defaults ────────────────────────────────────────────

#[test]
fn rtu_default_baud_rate() {
    let config = RtuConfig::default();
    assert_eq!(config.baud_rate, 9600);
}

#[test]
fn rtu_default_data_bits() {
    let config = RtuConfig::default();
    assert_eq!(config.data_bits, DataBits::Eight);
}

#[test]
fn rtu_default_parity() {
    let config = RtuConfig::default();
    assert_eq!(config.parity, Parity::None);
}

#[test]
fn rtu_default_stop_bits() {
    let config = RtuConfig::default();
    assert_eq!(config.stop_bits, StopBits::One);
}

// ── Interframe Delay Calculation ───────────────────────────────────

#[test]
fn interframe_delay_9600_baud() {
    // At 9600 baud: 3.5 × 11 / 9600 ≈ 4.01 ms
    // 11 bits per character = start(1) + data(8) + parity(0-1) + stop(1-2)
    let config = RtuConfig::default(); // 9600 baud
    let delay = config.interframe_delay();
    let us = delay.as_micros();
    // Should be approximately 4010 µs
    assert!((3900..=4100).contains(&us), "9600 baud delay: {us} µs");
}

#[test]
fn interframe_delay_19200_baud_still_calculated() {
    // At 19200: 3.5 × 11 / 19200 ≈ 2.005 ms — still uses formula
    let config = RtuConfig {
        baud_rate: 19200,
        ..RtuConfig::default()
    };
    let delay = config.interframe_delay();
    let us = delay.as_micros();
    assert!((1900..=2100).contains(&us), "19200 baud delay: {us} µs");
}

#[test]
fn interframe_delay_above_19200_fixed_1750us() {
    // Per Modbus serial spec: at baud rates > 19200, fixed 1.75 ms
    for baud in [38400, 57600, 115200, 230400] {
        let config = RtuConfig {
            baud_rate: baud,
            ..RtuConfig::default()
        };
        assert_eq!(
            config.interframe_delay(),
            Duration::from_micros(1750),
            "baud {baud} should use fixed 1750 µs"
        );
    }
}

#[test]
fn interframe_delay_boundary_19200_uses_formula() {
    // Exactly 19200 should still use the calculated formula, not fixed
    let config = RtuConfig {
        baud_rate: 19200,
        ..RtuConfig::default()
    };
    let delay = config.interframe_delay();
    // Should NOT be exactly 1750 µs (that's the fixed value)
    assert_ne!(delay, Duration::from_micros(1750));
}

#[test]
fn interframe_delay_19201_uses_fixed() {
    // Just above threshold — should use fixed
    let config = RtuConfig {
        baud_rate: 19201,
        ..RtuConfig::default()
    };
    assert_eq!(config.interframe_delay(), Duration::from_micros(1750));
}

// ── RTU-over-TCP Integration ──────────────────────────────────────

#[tokio::test]
async fn rtu_over_tcp_round_trip() {
    use bytes::Bytes;
    use futures_util::{SinkExt, StreamExt};
    use rusty_modbus_frame::frame::{Frame, FrameHeader};
    use rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;
    use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
    use tokio::net::TcpListener;
    use tokio_util::codec::Framed;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Server echoes one frame
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut framed = Framed::new(stream, RtuOverTcpCodec);
        if let Some(Ok(frame)) = framed.next().await {
            let _ = framed.send(frame).await;
        }
    });

    let config = rusty_modbus_tcp::config::TcpConfig::default();
    let (mut sink, mut stream) =
        rusty_modbus_rtu::rtu_tcp::RtuOverTcpTransport::connect(addr, config)
            .await
            .unwrap();

    // Send FC03 request as RTU frame
    let frame = Frame {
        header: FrameHeader::Rtu { unit_id: 0x01 },
        pdu: Bytes::from_static(&[0x03, 0x00, 0x6B, 0x00, 0x03]),
    };
    sink.send(frame).await.unwrap();

    let resp = stream.recv().await.unwrap();
    assert_eq!(resp.unit_id(), 0x01);
    assert_eq!(&resp.pdu[..], &[0x03, 0x00, 0x6B, 0x00, 0x03]);

    server.await.unwrap();
}
