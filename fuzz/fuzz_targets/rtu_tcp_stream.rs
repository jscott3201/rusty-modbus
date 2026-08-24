#![no_main]
#![forbid(unsafe_code)]

mod support;

use libfuzzer_sys::fuzz_target;
use rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;
use rusty_modbus_types::MAX_RTU_ADU_SIZE;

use support::{decode_incremental, validate_rtu_encoding};

fuzz_target!(|input: &[u8]| {
    decode_incremental(RtuOverTcpCodec, input, MAX_RTU_ADU_SIZE, |frame| {
        // Re-decoding an encoded frame can select an earlier CRC-valid prefix.
        // PR-104 owns that policy; this target preserves it and checks the
        // bounded encoded frame and terminal CRC instead.
        validate_rtu_encoding::<RtuOverTcpCodec>(frame, MAX_RTU_ADU_SIZE);
    });
});
