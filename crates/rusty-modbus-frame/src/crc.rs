//! CRC-16/Modbus computation for RTU framing.
//!
//! Polynomial: 0xA001 (reflected 0x8005), init: 0xFFFF.
//! Stored little-endian on the wire (low byte first).

/// CRC-16/Modbus lookup table (256 entries, computed at compile time).
const CRC_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut i = 0u16;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// Compute CRC-16/Modbus over the given bytes.
#[must_use]
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc = crc16_update(crc, byte);
    }
    crc
}

pub(crate) fn crc16_update(crc: u16, byte: u8) -> u16 {
    let index = (crc ^ u16::from(byte)) & 0xFF;
    (crc >> 8) ^ CRC_TABLE[index as usize]
}

/// Verify that the last 2 bytes of `frame` are a valid CRC of the preceding bytes.
///
/// Returns `false` if the frame is shorter than 3 bytes (minimum: 1 data byte + 2 CRC).
#[must_use]
pub fn verify_crc(frame: &[u8]) -> bool {
    if frame.len() < 3 {
        return false;
    }
    let data_end = frame.len() - 2;
    let expected = crc16(&frame[..data_end]);
    let actual = u16::from_le_bytes([frame[data_end], frame[data_end + 1]]);
    expected == actual
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector_modbus_rtu() {
        // Common test vector: slave address 0x01, FC 0x03, start 0x0000, qty 0x000A
        // On wire (little-endian): [0xC5, 0xCD] → native u16 = 0xCDC5
        let data = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A];
        let crc = crc16(&data);
        assert_eq!(crc, 0xCDC5, "CRC mismatch: got {crc:#06X}");
    }

    #[test]
    fn known_vector_simple() {
        // "123456789" in ASCII → CRC-16/Modbus = 0x4B37
        let data = b"123456789";
        let crc = crc16(data);
        assert_eq!(crc, 0x4B37, "CRC mismatch: got {crc:#06X}");
    }

    #[test]
    fn verify_crc_valid() {
        let data = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A];
        let crc = crc16(&data);
        let crc_bytes = crc.to_le_bytes();
        let frame = [&data[..], &crc_bytes[..]].concat();
        assert!(verify_crc(&frame));
    }

    #[test]
    fn verify_crc_invalid() {
        let frame = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00];
        assert!(!verify_crc(&frame));
    }

    #[test]
    fn verify_crc_too_short() {
        assert!(!verify_crc(&[0x01, 0x02]));
        assert!(!verify_crc(&[]));
    }

    #[test]
    fn empty_data() {
        let crc = crc16(&[]);
        assert_eq!(crc, 0xFFFF, "empty data should return init value");
    }

    #[test]
    fn single_byte() {
        // CRC of single zero byte
        let crc = crc16(&[0x00]);
        // Verify it's deterministic and non-trivial
        assert_ne!(crc, 0xFFFF);
        assert_ne!(crc, 0x0000);
    }
}
