#![no_main]
#![forbid(unsafe_code)]

mod support;

use libfuzzer_sys::fuzz_target;
use rusty_modbus_frame::MbapCodec;
use rusty_modbus_types::MAX_TCP_ADU_SIZE;

use support::{decode_incremental, round_trip_complete};

fuzz_target!(|input: &[u8]| {
    decode_incremental(MbapCodec, input, MAX_TCP_ADU_SIZE, |frame| {
        round_trip_complete::<MbapCodec>(frame, MAX_TCP_ADU_SIZE);
    });
});
