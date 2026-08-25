#![no_main]
#![forbid(unsafe_code)]

mod support;

use libfuzzer_sys::fuzz_target;
use rusty_modbus_frame::rtu_tcp::{
    RtuOverTcpCodec, RtuOverTcpDirection, RtuOverTcpFramingPolicy,
};
use rusty_modbus_types::MAX_RTU_ADU_SIZE;

use support::{decode_incremental, validate_rtu_encoding};

fuzz_target!(|input: &[u8]| {
    decode_incremental(RtuOverTcpCodec, input, MAX_RTU_ADU_SIZE, |frame| {
        // Compatibility intentionally preserves first-valid-prefix selection,
        // so only the bounded encoded frame and terminal CRC are checked.
        validate_rtu_encoding::<RtuOverTcpCodec>(frame, MAX_RTU_ADU_SIZE);
    });

    for direction in [
        RtuOverTcpDirection::Request,
        RtuOverTcpDirection::Response,
    ] {
        let strict = RtuOverTcpCodec::with_policy(
            RtuOverTcpFramingPolicy::FunctionAwareStrict,
            direction,
        );
        decode_incremental(strict, input, MAX_RTU_ADU_SIZE, |frame| {
            validate_rtu_encoding::<RtuOverTcpCodec>(frame, MAX_RTU_ADU_SIZE);
        });
    }
});
