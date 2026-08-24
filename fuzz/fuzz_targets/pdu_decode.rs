#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use rusty_modbus_codec::{decode_pdu_ref, decode_request, decode_response};
use rusty_modbus_types::MAX_PDU_SIZE;

fuzz_target!(|input: &[u8]| {
    // One byte beyond the protocol ceiling keeps the oversized branch reachable
    // without letting the fuzzer control work or allocation size.
    let pdu = &input[..input.len().min(MAX_PDU_SIZE + 1)];

    let raw = decode_pdu_ref(pdu);
    let request = decode_request(pdu);
    let response = decode_response(pdu);

    if let Ok(raw) = raw {
        assert!(!pdu.is_empty());
        assert!(pdu.len() <= MAX_PDU_SIZE);
        assert_eq!(raw.function_code, pdu[0]);
        assert_eq!(raw.data, &pdu[1..]);
    }
    if request.is_ok() || response.is_ok() {
        assert!(raw.is_ok());
    }
});
