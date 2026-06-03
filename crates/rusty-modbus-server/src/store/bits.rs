//! Packed bit-table storage for in-memory coils and discrete inputs.

use rusty_modbus_types::ExceptionCode;

pub(super) struct BitTable {
    len: usize,
    bytes: Vec<u8>,
}

impl BitTable {
    pub(super) fn new(len: usize) -> Self {
        Self {
            len,
            bytes: vec![0; len.div_ceil(8)],
        }
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn set(&mut self, index: usize, value: bool) {
        let mask = 1 << (index % 8);
        let byte = &mut self.bytes[index / 8];
        if value {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
    }

    pub(super) fn read_bits(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [bool],
    ) -> Result<usize, ExceptionCode> {
        let start = usize::from(address);
        let quantity = usize::from(quantity);
        check_range(address, quantity, self.len)?;
        if out.len() < quantity {
            return Err(ExceptionCode::IllegalDataValue);
        }
        for byte_index in 0..quantity.div_ceil(8) {
            let out_start = byte_index * 8;
            let bit_count = (quantity - out_start).min(8);
            let byte = self.read_byte_bits(start + out_start, bit_count);
            for bit in 0..bit_count {
                out[out_start + bit] = (byte >> bit) & 1 == 1;
            }
        }
        Ok(quantity)
    }

    pub(super) fn write_bits(
        &mut self,
        address: u16,
        values: &[bool],
    ) -> Result<(), ExceptionCode> {
        let start = usize::from(address);
        check_range(address, values.len(), self.len)?;
        for (byte_index, chunk) in values.chunks(8).enumerate() {
            let mut byte = 0u8;
            for (bit, &value) in chunk.iter().enumerate() {
                byte |= u8::from(value) << bit;
            }
            self.write_byte_bits(start + byte_index * 8, byte, chunk.len());
        }
        Ok(())
    }

    pub(super) fn read_packed(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [u8],
    ) -> Result<usize, ExceptionCode> {
        let start = usize::from(address);
        let quantity = usize::from(quantity);
        check_range(address, quantity, self.len)?;
        let byte_count = quantity.div_ceil(8);
        if out.len() < byte_count {
            return Err(ExceptionCode::IllegalDataValue);
        }
        for (byte_index, out_byte) in out[..byte_count].iter_mut().enumerate() {
            let source_bit = start + byte_index * 8;
            let bit_count = (quantity - byte_index * 8).min(8);
            *out_byte = self.read_byte_bits(source_bit, bit_count);
        }
        Ok(quantity)
    }

    pub(super) fn write_packed(
        &mut self,
        address: u16,
        quantity: usize,
        packed_values: &[u8],
    ) -> Result<(), ExceptionCode> {
        let start = usize::from(address);
        check_range(address, quantity, self.len)?;
        for (byte_index, &byte) in packed_values.iter().enumerate() {
            let offset = byte_index * 8;
            let bit_count = (quantity - offset).min(8);
            self.write_byte_bits(start + offset, byte, bit_count);
        }
        Ok(())
    }

    fn read_byte_bits(&self, start: usize, bit_count: usize) -> u8 {
        debug_assert!((1..=8).contains(&bit_count));
        let byte_index = start / 8;
        let shift = start % 8;
        let mut byte = self.bytes[byte_index] >> shift;
        if shift != 0 && byte_index + 1 < self.bytes.len() {
            byte |= self.bytes[byte_index + 1] << (8 - shift);
        }
        byte & low_bit_mask(bit_count)
    }

    fn write_byte_bits(&mut self, start: usize, value: u8, bit_count: usize) {
        debug_assert!((1..=8).contains(&bit_count));
        let byte_index = start / 8;
        let shift = start % 8;
        let mask = low_bit_mask(bit_count);
        let value = value & mask;

        if shift + bit_count <= 8 {
            let shifted_mask = mask << shift;
            self.bytes[byte_index] = (self.bytes[byte_index] & !shifted_mask) | (value << shift);
            return;
        }

        let low_bits = 8 - shift;
        let low_mask = low_bit_mask(low_bits) << shift;
        self.bytes[byte_index] =
            (self.bytes[byte_index] & !low_mask) | ((value << shift) & low_mask);

        let high_bits = bit_count - low_bits;
        let high_mask = low_bit_mask(high_bits);
        self.bytes[byte_index + 1] =
            (self.bytes[byte_index + 1] & !high_mask) | ((value >> low_bits) & high_mask);
    }
}

fn check_range(address: u16, quantity: usize, max: usize) -> Result<(), ExceptionCode> {
    let end = usize::from(address)
        .checked_add(quantity)
        .ok_or(ExceptionCode::IllegalDataAddress)?;
    if end > max {
        return Err(ExceptionCode::IllegalDataAddress);
    }
    Ok(())
}

fn low_bit_mask(bit_count: usize) -> u8 {
    if bit_count >= 8 {
        u8::MAX
    } else {
        (1 << bit_count) - 1
    }
}
