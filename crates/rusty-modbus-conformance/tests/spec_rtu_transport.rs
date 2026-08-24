//! RTU Transport conformance tests.
//!
//! Verifies RTU configuration and timing against the Modbus serial
//! line specification.

use std::time::Duration;

use rusty_modbus_rtu::config::{
    DataBits, Parity, RtuConfig, RtuSerialFormat, RtuTimingMode, StopBits, StrictRtuConfig,
};
use rusty_modbus_rtu::{RtuConfigError, RtuUnitIdRole};

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

#[test]
fn legacy_rtu_default_timeout_and_timing_remain_compatible() {
    let config = RtuConfig::default();

    assert_eq!(config.response_timeout, Duration::from_secs(1));
    assert_eq!(
        config.intercharacter_timeout(),
        Duration::from_nanos(1_718_750)
    );
    assert_eq!(config.interframe_delay(), Duration::from_nanos(4_010_417));
}

#[test]
fn strict_rtu_default_resolves_to_19200_8e1() {
    let config = StrictRtuConfig::default();
    let resolved = config.resolve();

    assert_eq!(config.baud_rate(), 19_200);
    assert_eq!(config.serial_format(), RtuSerialFormat::EightEvenOne);
    assert_eq!(config.response_timeout(), Duration::from_secs(1));
    assert_eq!(resolved.data_bits(), DataBits::Eight);
    assert_eq!(resolved.parity(), Parity::Even);
    assert_eq!(resolved.stop_bits(), StopBits::One);
    assert_eq!(resolved.timing_mode(), RtuTimingMode::CharacterCalculated);
}

#[test]
fn strict_rtu_accepts_only_8e1_8o1_and_8n2_raw_formats() {
    let cases = [
        (Parity::Even, StopBits::One, RtuSerialFormat::EightEvenOne),
        (Parity::Odd, StopBits::One, RtuSerialFormat::EightOddOne),
        (Parity::None, StopBits::Two, RtuSerialFormat::EightNoneTwo),
    ];
    for (parity, stop_bits, format) in cases {
        let raw = RtuConfig {
            baud_rate: 9_600,
            data_bits: DataBits::Eight,
            parity,
            stop_bits,
            response_timeout: Duration::from_secs(1),
        };
        assert_eq!(
            StrictRtuConfig::try_from(&raw).unwrap().serial_format(),
            format
        );
    }

    let invalid = [
        (DataBits::Five, Parity::Even, StopBits::One),
        (DataBits::Six, Parity::Even, StopBits::One),
        (DataBits::Seven, Parity::Even, StopBits::One),
        (DataBits::Eight, Parity::None, StopBits::One),
        (DataBits::Eight, Parity::Even, StopBits::Two),
        (DataBits::Eight, Parity::Odd, StopBits::Two),
    ];
    for (data_bits, parity, stop_bits) in invalid {
        let raw = RtuConfig {
            baud_rate: 9_600,
            data_bits,
            parity,
            stop_bits,
            response_timeout: Duration::from_secs(1),
        };
        assert!(matches!(
            StrictRtuConfig::try_from(&raw),
            Err(RtuConfigError::InvalidSerialFormat { .. })
        ));
    }

    let zero_baud = RtuConfig {
        baud_rate: 0,
        data_bits: DataBits::Eight,
        parity: Parity::Even,
        stop_bits: StopBits::One,
        response_timeout: Duration::from_secs(1),
    };
    assert_eq!(
        StrictRtuConfig::try_from(&zero_baud),
        Err(RtuConfigError::ZeroBaudRate)
    );
}

#[test]
fn strict_rtu_timing_uses_exact_ceiling_and_high_speed_recommendation() {
    let at_9600 =
        StrictRtuConfig::new(9_600, RtuSerialFormat::EightEvenOne, Duration::from_secs(1))
            .unwrap()
            .resolve();
    assert_eq!(at_9600.character_time(), Duration::from_nanos(1_145_834));
    assert_eq!(at_9600.t1_5(), Duration::from_nanos(1_718_750));
    assert_eq!(at_9600.t3_5(), Duration::from_nanos(4_010_417));

    let irregular =
        StrictRtuConfig::new(12_345, RtuSerialFormat::EightOddOne, Duration::from_secs(1))
            .unwrap()
            .resolve();
    assert_eq!(irregular.character_time(), Duration::from_nanos(891_050));
    assert_eq!(irregular.t1_5(), Duration::from_nanos(1_336_574));
    assert_eq!(irregular.t3_5(), Duration::from_nanos(3_118_672));

    let high_speed = StrictRtuConfig::new(
        19_201,
        RtuSerialFormat::EightNoneTwo,
        Duration::from_secs(1),
    )
    .unwrap()
    .resolve();
    assert_eq!(high_speed.t1_5(), Duration::from_micros(750));
    assert_eq!(high_speed.t3_5(), Duration::from_micros(1_750));
    assert_eq!(
        high_speed.timing_mode(),
        RtuTimingMode::FixedHighSpeedRecommendation
    );

    let maximum = StrictRtuConfig::new(
        u32::MAX,
        RtuSerialFormat::EightEvenOne,
        Duration::from_secs(1),
    )
    .unwrap()
    .resolve();
    assert_eq!(maximum.character_time(), Duration::from_nanos(3));
}

#[test]
fn strict_rtu_unit_id_roles_enforce_serial_address_classes() {
    for unit_id in [0, 1, 247] {
        assert_eq!(RtuUnitIdRole::ClientDestination.validate(unit_id), Ok(()));
    }
    for unit_id in [248, 255] {
        assert!(RtuUnitIdRole::ClientDestination.validate(unit_id).is_err());
    }

    for unit_id in [1, 247] {
        assert_eq!(RtuUnitIdRole::ResponderSource.validate(unit_id), Ok(()));
    }
    for unit_id in [0, 248, 255] {
        assert!(RtuUnitIdRole::ResponderSource.validate(unit_id).is_err());
    }
}

// ── Interframe Delay Calculation ───────────────────────────────────

#[test]
fn interframe_delay_9600_baud() {
    // At 9600 baud: 3.5 × 11 / 9600 ≈ 4.01 ms
    // The compatibility method assumes 11 bits despite its raw 8N1 default.
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
    // The serial-line guide recommends fixed 1.75 ms above 19200 baud.
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

// ── Inter-character Timeout Calculation ───────────────────────────

#[test]
fn intercharacter_timeout_9600_baud() {
    // At 9600 baud: 1.5 × 11 / 9600 ≈ 1.719 ms.
    let config = RtuConfig::default();
    let timeout = config.intercharacter_timeout();
    let us = timeout.as_micros();
    assert!((1700..=1730).contains(&us), "9600 baud timeout: {us} µs");
}

#[test]
fn intercharacter_timeout_19200_baud_still_calculated() {
    // At 19200: 1.5 × 11 / 19200 ≈ 859 µs — still uses formula.
    let config = RtuConfig {
        baud_rate: 19200,
        ..RtuConfig::default()
    };
    let timeout = config.intercharacter_timeout();
    let us = timeout.as_micros();
    assert!((840..=880).contains(&us), "19200 baud timeout: {us} µs");
}

#[test]
fn intercharacter_timeout_above_19200_fixed_750us() {
    // The serial-line guide recommends fixed 750 µs above 19200 baud.
    for baud in [38400, 57600, 115200, 230400] {
        let config = RtuConfig {
            baud_rate: baud,
            ..RtuConfig::default()
        };
        assert_eq!(
            config.intercharacter_timeout(),
            Duration::from_micros(750),
            "baud {baud} should use fixed 750 µs"
        );
    }
}

#[test]
fn intercharacter_timeout_boundary_19200_uses_formula() {
    let config = RtuConfig {
        baud_rate: 19200,
        ..RtuConfig::default()
    };

    assert_ne!(config.intercharacter_timeout(), Duration::from_micros(750));
}

#[test]
fn intercharacter_timeout_19201_uses_fixed() {
    let config = RtuConfig {
        baud_rate: 19201,
        ..RtuConfig::default()
    };

    assert_eq!(config.intercharacter_timeout(), Duration::from_micros(750));
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
