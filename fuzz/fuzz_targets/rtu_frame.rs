#![no_main]
#![forbid(unsafe_code)]

mod support;

use libfuzzer_sys::fuzz_target;
use rusty_modbus_frame::rtu::RtuCodec;
use rusty_modbus_types::MAX_RTU_ADU_SIZE;

use support::{decode_complete, round_trip_complete};

fuzz_target!(|input: &[u8]| {
    decode_complete(RtuCodec, input, MAX_RTU_ADU_SIZE + 1, |frame| {
        round_trip_complete::<RtuCodec>(frame, MAX_RTU_ADU_SIZE);
    });
});
