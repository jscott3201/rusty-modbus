//! Shared validation for FC 0x14/0x15 file-record references.

use rusty_modbus_types::ExceptionCode;

/// First valid file number in the Modbus file-record model.
pub(crate) const MIN_FILE_NUMBER: u16 = 1;
/// Last valid record number in a Modbus file.
pub(crate) const MAX_RECORD_NUMBER: u16 = 0x270F;
/// Number of records in a Modbus file.
pub(crate) const RECORD_COUNT: usize = 0x2710;

/// Validate a file-record reference against V1.1b3 §6.14/§6.15.
pub(crate) fn validate_range(
    file_number: u16,
    record_number: u16,
    record_length: usize,
) -> Result<(), ExceptionCode> {
    let end = usize::from(record_number)
        .checked_add(record_length)
        .ok_or(ExceptionCode::IllegalDataAddress)?;
    if file_number < MIN_FILE_NUMBER
        || record_length == 0
        || record_number > MAX_RECORD_NUMBER
        || end > RECORD_COUNT
    {
        return Err(ExceptionCode::IllegalDataAddress);
    }
    Ok(())
}
