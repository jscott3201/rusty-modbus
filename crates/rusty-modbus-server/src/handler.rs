//! Request dispatch and response building.

use rusty_modbus_codec::request::{ReadFileRecordRequest, WriteFileRecordRequest};
use rusty_modbus_codec::response::{
    GetCommEventCounterResponse, MaskWriteRegisterResponse, ReadExceptionStatusResponse,
    WriteFileRecordResponse, WriteMultipleCoilsResponse, WriteMultipleRegistersResponse,
    WriteSingleCoilResponse, WriteSingleRegisterResponse,
};
use rusty_modbus_codec::{DecodeError, RequestPdu, decode_request, validate};
use rusty_modbus_types::{
    Address, DiagnosticSubFunction, ExceptionCode, FunctionCode, MAX_FIFO_VALUES, MAX_PDU_SIZE,
    MeiType, Quantity, UnitId,
};
use tracing::debug;

use crate::config::DeviceIdentification;
use crate::device_id::build_device_id_response;
use crate::file_record;
use crate::response_encode::encode_response;
use crate::store::{
    DataStore, MAX_COMM_EVENT_LOG_EVENTS, MAX_DIAGNOSTIC_RESPONSE_DATA_LEN,
    MAX_FILE_RECORD_REGISTERS, MAX_SERVER_ID_BYTES,
};

/// Process a request PDU and return a response PDU (or `None` for broadcast writes).
///
/// The `pdu` slice starts at the function code byte.
#[allow(clippy::too_many_lines)]
#[tracing::instrument(
    level = "trace",
    skip(pdu, store, device_id),
    fields(
        unit_id = unit_id.0,
        pdu_len = pdu.len(),
        function_code = pdu.first().copied().unwrap_or_default()
    )
)]
pub async fn process_request<S: DataStore>(
    pdu: &[u8],
    unit_id: UnitId,
    store: &S,
    device_id: &DeviceIdentification,
) -> Option<Vec<u8>> {
    let is_broadcast = unit_id.is_broadcast();

    let request = match decode_request(pdu) {
        Ok(req) => req,
        Err(e) => {
            if is_broadcast {
                return None;
            }
            let fc = pdu.first().copied().unwrap_or(0);
            // Per spec state diagrams (V1.1b3 Figures 11-28):
            // - Unknown function code → IllegalFunction (0x01)
            // - Known function code with bad data → IllegalDataValue (0x03)
            let exc = match e {
                // Unknown function code, and an unrecognized Diagnostics (FC 0x08)
                // sub-function, are both illegal *functions* — not illegal data
                // values (V1.1b3 §4.5 and §6.8, Figure 18).
                DecodeError::UnknownFunctionCode(_)
                | DecodeError::UnknownDiagnosticSubFunction(_) => ExceptionCode::IllegalFunction,
                DecodeError::InvalidReferenceType(_) | DecodeError::FileRecordOutOfRange { .. } => {
                    ExceptionCode::IllegalDataAddress
                }
                _ => {
                    // FC is recognized (otherwise decode would return UnknownFunctionCode),
                    // but the data is malformed (truncated, bad quantity, bad byte count,
                    // invalid coil value, etc.) → IllegalDataValue per spec §4.5
                    ExceptionCode::IllegalDataValue
                }
            };
            debug!(
                function_code = fc,
                exception_code = exc.code(),
                error = %e,
                "request decode failed; returning Modbus exception"
            );
            return Some(encode_exception(fc | 0x80, exc));
        }
    };

    dispatch_request(request, pdu, is_broadcast, store, device_id).await
}

#[allow(clippy::too_many_lines)]
async fn dispatch_request<S: DataStore>(
    request: RequestPdu<'_>,
    pdu: &[u8],
    is_broadcast: bool,
    store: &S,
    device_id: &DeviceIdentification,
) -> Option<Vec<u8>> {
    match request {
        RequestPdu::ReadHoldingRegisters(req) => {
            if is_broadcast {
                return None;
            }
            Some(
                handle_read_registers(
                    FunctionCode::ReadHoldingRegisters,
                    req.address,
                    req.quantity,
                    store,
                    true,
                )
                .await,
            )
        }
        RequestPdu::ReadInputRegisters(req) => {
            if is_broadcast {
                return None;
            }
            Some(
                handle_read_registers(
                    FunctionCode::ReadInputRegisters,
                    req.address,
                    req.quantity,
                    store,
                    false,
                )
                .await,
            )
        }
        RequestPdu::ReadCoils(req) => {
            if is_broadcast {
                return None;
            }
            Some(
                handle_read_bits(
                    FunctionCode::ReadCoils,
                    req.address,
                    req.quantity,
                    store,
                    true,
                )
                .await,
            )
        }
        RequestPdu::ReadDiscreteInputs(req) => {
            if is_broadcast {
                return None;
            }
            Some(
                handle_read_bits(
                    FunctionCode::ReadDiscreteInputs,
                    req.address,
                    req.quantity,
                    store,
                    false,
                )
                .await,
            )
        }
        RequestPdu::WriteSingleRegister(req) => {
            let result = store.write_register(req.address.0, req.value).await;
            if is_broadcast {
                return None;
            }
            Some(match result {
                Ok(()) => encode_response(&WriteSingleRegisterResponse {
                    address: req.address,
                    value: req.value,
                }),
                Err(ec) => encode_exception(FunctionCode::WriteSingleRegister.exception_code(), ec),
            })
        }
        RequestPdu::WriteMultipleRegisters(req) => {
            if let Err(ec) =
                validate::validate_write_registers(req.address.0, req.quantity.0, req.byte_count)
            {
                if is_broadcast {
                    return None;
                }
                return Some(encode_exception(
                    FunctionCode::WriteMultipleRegisters.exception_code(),
                    ec,
                ));
            }
            let result = store
                .write_registers_be(req.address.0, req.quantity.0, req.register_values)
                .await;
            if is_broadcast {
                return None;
            }
            Some(match result {
                Ok(()) => encode_response(&WriteMultipleRegistersResponse {
                    address: req.address,
                    quantity: req.quantity,
                }),
                Err(ec) => {
                    encode_exception(FunctionCode::WriteMultipleRegisters.exception_code(), ec)
                }
            })
        }
        RequestPdu::WriteSingleCoil(req) => {
            let result = store.write_coil(req.address.0, req.value.as_bool()).await;
            if is_broadcast {
                return None;
            }
            Some(match result {
                Ok(()) => encode_response(&WriteSingleCoilResponse {
                    address: req.address,
                    value: req.value,
                }),
                Err(ec) => encode_exception(FunctionCode::WriteSingleCoil.exception_code(), ec),
            })
        }
        RequestPdu::WriteMultipleCoils(req) => {
            if let Err(ec) =
                validate::validate_write_coils(req.address.0, req.quantity.0, req.byte_count)
            {
                if is_broadcast {
                    return None;
                }
                return Some(encode_exception(
                    FunctionCode::WriteMultipleCoils.exception_code(),
                    ec,
                ));
            }
            let result = store
                .write_coils_packed(req.address.0, req.quantity.0, req.coil_values)
                .await;
            if is_broadcast {
                return None;
            }
            Some(match result {
                Ok(()) => encode_response(&WriteMultipleCoilsResponse {
                    address: req.address,
                    quantity: req.quantity,
                }),
                Err(ec) => encode_exception(FunctionCode::WriteMultipleCoils.exception_code(), ec),
            })
        }
        RequestPdu::MaskWriteRegister(req) => {
            let result = handle_mask_write(req.address, req.and_mask, req.or_mask, store).await;
            if is_broadcast {
                return None;
            }
            Some(match result {
                Ok(()) => encode_response(&MaskWriteRegisterResponse {
                    address: req.address,
                    and_mask: req.and_mask,
                    or_mask: req.or_mask,
                }),
                Err(ec) => encode_exception(FunctionCode::MaskWriteRegister.exception_code(), ec),
            })
        }
        RequestPdu::ReadWriteMultipleRegisters(req) => {
            if is_broadcast {
                return None;
            }
            Some(handle_read_write_multiple(req, store).await)
        }
        RequestPdu::ReadFileRecord(req) => {
            if is_broadcast {
                return None;
            }
            Some(handle_read_file_record(req, store).await)
        }
        RequestPdu::WriteFileRecord(req) => {
            let result = apply_write_file_record(&req, store).await;
            if is_broadcast {
                return None;
            }
            Some(match result {
                // Per §6.15 a successful write echoes the request sub-requests verbatim.
                Ok(()) => encode_response(&WriteFileRecordResponse {
                    byte_count: req.byte_count,
                    data: req.sub_requests,
                }),
                Err(ec) => encode_exception(FunctionCode::WriteFileRecord.exception_code(), ec),
            })
        }
        RequestPdu::ReadFifoQueue(req) => {
            if is_broadcast {
                return None;
            }
            Some(handle_read_fifo_queue(req.fifo_pointer_address, store).await)
        }
        RequestPdu::ReadExceptionStatus => {
            if is_broadcast {
                return None;
            }
            Some(match store.read_exception_status().await {
                Ok(status) => encode_response(&ReadExceptionStatusResponse { status }),
                Err(ec) => encode_exception(FunctionCode::ReadExceptionStatus.exception_code(), ec),
            })
        }
        RequestPdu::Diagnostics(req) => {
            // Run the sub-function first (it may mutate device state, e.g. clear
            // counters), then suppress the reply on broadcast — mirroring the
            // write arms. Ok(None) is the spec "no reply" path (Force Listen Only).
            let result = handle_diagnostics(req.sub_function, req.data, store).await;
            if is_broadcast {
                return None;
            }
            result
        }
        RequestPdu::GetCommEventCounter => {
            if is_broadcast {
                return None;
            }
            Some(match store.get_comm_event_counter().await {
                Ok((status, event_count)) => encode_response(&GetCommEventCounterResponse {
                    status,
                    event_count,
                }),
                Err(ec) => encode_exception(FunctionCode::GetCommEventCounter.exception_code(), ec),
            })
        }
        RequestPdu::GetCommEventLog => {
            if is_broadcast {
                return None;
            }
            Some(handle_comm_event_log(store).await)
        }
        RequestPdu::ReportServerId => {
            if is_broadcast {
                return None;
            }
            Some(handle_report_server_id(store).await)
        }
        RequestPdu::EncapsulatedInterface(req) => {
            if is_broadcast {
                return None;
            }
            if req.mei_type == MeiType::ReadDeviceIdentification {
                Some(build_device_id_response(req.data, device_id))
            } else {
                let fc = pdu.first().copied().unwrap_or(0);
                Some(encode_exception(fc | 0x80, ExceptionCode::IllegalFunction))
            }
        }
        RequestPdu::Custom(..) => {
            if is_broadcast {
                return None;
            }
            let fc = pdu.first().copied().unwrap_or(0);
            Some(encode_exception(fc | 0x80, ExceptionCode::IllegalFunction))
        }
    }
}

async fn handle_comm_event_log<S: DataStore>(store: &S) -> Vec<u8> {
    let mut response = Vec::with_capacity(8 + MAX_COMM_EVENT_LOG_EVENTS);
    response.push(FunctionCode::GetCommEventLog.code());
    response.push(0);
    response.extend_from_slice(&[0; 6]);

    let events_start = response.len();
    let result = store.append_comm_event_log(&mut response).await;
    match result {
        Ok(meta) => {
            let events_len = response.len().saturating_sub(events_start);
            if events_len > MAX_COMM_EVENT_LOG_EVENTS {
                return encode_exception(
                    FunctionCode::GetCommEventLog.exception_code(),
                    ExceptionCode::ServerDeviceFailure,
                );
            }
            let byte_count =
                match checked_response_u8(events_len + 6, FunctionCode::GetCommEventLog) {
                    Ok(byte_count) => byte_count,
                    Err(resp) => return resp,
                };
            response[1] = byte_count;
            response[2..4].copy_from_slice(&meta.status.to_be_bytes());
            response[4..6].copy_from_slice(&meta.event_count.to_be_bytes());
            response[6..8].copy_from_slice(&meta.message_count.to_be_bytes());
            response
        }
        Err(ec) => encode_exception(FunctionCode::GetCommEventLog.exception_code(), ec),
    }
}

async fn handle_diagnostics<S: DataStore>(
    sub_function: DiagnosticSubFunction,
    data: &[u8],
    store: &S,
) -> Option<Vec<u8>> {
    let mut response = Vec::with_capacity(3 + MAX_DIAGNOSTIC_RESPONSE_DATA_LEN);
    response.push(FunctionCode::Diagnostics.code());
    response.extend_from_slice(&sub_function.code().to_be_bytes());

    let data_start = response.len();
    let result = store
        .append_diagnostic_response(sub_function, data, &mut response)
        .await;
    match result {
        Ok(Some(count)) => {
            let actual_count = response.len().saturating_sub(data_start);
            if count != actual_count
                || actual_count > MAX_DIAGNOSTIC_RESPONSE_DATA_LEN
                || !actual_count.is_multiple_of(2)
            {
                return Some(encode_exception(
                    FunctionCode::Diagnostics.exception_code(),
                    ExceptionCode::ServerDeviceFailure,
                ));
            }
            Some(response)
        }
        Ok(None) => None,
        Err(ec) => Some(encode_exception(
            FunctionCode::Diagnostics.exception_code(),
            ec,
        )),
    }
}

async fn handle_report_server_id<S: DataStore>(store: &S) -> Vec<u8> {
    let mut response = Vec::with_capacity(2 + MAX_SERVER_ID_BYTES);
    response.push(FunctionCode::ReportServerId.code());
    response.push(0);

    let data_start = response.len();
    let result = store.append_server_id(&mut response).await;
    match result {
        Ok(count) => {
            let actual_count = response.len().saturating_sub(data_start);
            if count != actual_count || actual_count > MAX_SERVER_ID_BYTES {
                return encode_exception(
                    FunctionCode::ReportServerId.exception_code(),
                    ExceptionCode::ServerDeviceFailure,
                );
            }
            let byte_count = match checked_response_u8(actual_count, FunctionCode::ReportServerId) {
                Ok(byte_count) => byte_count,
                Err(resp) => return resp,
            };
            response[1] = byte_count;
            response
        }
        Err(ec) => encode_exception(FunctionCode::ReportServerId.exception_code(), ec),
    }
}

async fn handle_read_registers<S: DataStore>(
    fc: FunctionCode,
    address: Address,
    quantity: Quantity,
    store: &S,
    is_holding: bool,
) -> Vec<u8> {
    if let Err(ec) = validate::validate_read_registers(address.0, quantity.0) {
        return encode_exception(fc.exception_code(), ec);
    }

    let byte_count = match checked_response_u8(usize::from(quantity.0) * 2, fc) {
        Ok(byte_count) => byte_count,
        Err(resp) => return resp,
    };
    let response_fc = if is_holding {
        FunctionCode::ReadHoldingRegisters
    } else {
        FunctionCode::ReadInputRegisters
    };
    let mut response = vec![0u8; 2 + usize::from(byte_count)];
    response[0] = response_fc.code();
    response[1] = byte_count;

    let result = if is_holding {
        store
            .read_holding_registers_be(address.0, quantity.0, &mut response[2..])
            .await
    } else {
        store
            .read_input_registers_be(address.0, quantity.0, &mut response[2..])
            .await
    };

    match result {
        Ok(count) => {
            if let Err(ec) = validate_store_count(count, quantity.0, usize::from(quantity.0)) {
                return encode_exception(fc.exception_code(), ec);
            }
            response
        }
        Err(ec) => encode_exception(fc.exception_code(), ec),
    }
}

async fn handle_read_bits<S: DataStore>(
    fc: FunctionCode,
    address: Address,
    quantity: Quantity,
    store: &S,
    is_coils: bool,
) -> Vec<u8> {
    let validation = if is_coils {
        validate::validate_read_coils(address.0, quantity.0)
    } else {
        validate::validate_read_discrete_inputs(address.0, quantity.0)
    };
    if let Err(ec) = validation {
        return encode_exception(fc.exception_code(), ec);
    }

    let byte_count = match checked_response_u8(usize::from(quantity.0).div_ceil(8), fc) {
        Ok(byte_count) => byte_count,
        Err(resp) => return resp,
    };
    let response_fc = if is_coils {
        FunctionCode::ReadCoils
    } else {
        FunctionCode::ReadDiscreteInputs
    };
    let mut response = vec![0u8; 2 + usize::from(byte_count)];
    response[0] = response_fc.code();
    response[1] = byte_count;

    let result = if is_coils {
        store
            .read_coils_packed(address.0, quantity.0, &mut response[2..])
            .await
    } else {
        store
            .read_discrete_inputs_packed(address.0, quantity.0, &mut response[2..])
            .await
    };

    match result {
        Ok(count) => {
            if let Err(ec) = validate_store_count(count, quantity.0, usize::from(quantity.0)) {
                return encode_exception(fc.exception_code(), ec);
            }
            response
        }
        Err(ec) => encode_exception(fc.exception_code(), ec),
    }
}

async fn handle_mask_write<S: DataStore>(
    address: Address,
    and_mask: u16,
    or_mask: u16,
    store: &S,
) -> Result<(), ExceptionCode> {
    validate::validate_mask_write_address(address.0)?;
    store
        .atomic_mask_write_register(address.0, and_mask, or_mask)
        .await
}

async fn handle_read_write_multiple<S: DataStore>(
    req: rusty_modbus_codec::request::ReadWriteMultipleRegistersRequest<'_>,
    store: &S,
) -> Vec<u8> {
    if let Err(ec) = validate::validate_read_write_registers(
        req.read_address.0,
        req.read_quantity.0,
        req.write_address.0,
        req.write_quantity.0,
        req.write_byte_count,
    ) {
        return encode_exception(
            FunctionCode::ReadWriteMultipleRegisters.exception_code(),
            ec,
        );
    }

    let byte_count = match checked_response_u8(
        usize::from(req.read_quantity.0) * 2,
        FunctionCode::ReadWriteMultipleRegisters,
    ) {
        Ok(byte_count) => byte_count,
        Err(resp) => return resp,
    };
    let mut response = vec![0u8; 2 + usize::from(byte_count)];
    response[0] = FunctionCode::ReadWriteMultipleRegisters.code();
    response[1] = byte_count;

    match store
        .atomic_read_write_registers_be(
            req.read_address.0,
            req.read_quantity.0,
            req.write_address.0,
            req.write_quantity.0,
            req.write_register_values,
            &mut response[2..],
        )
        .await
    {
        Ok(count) => {
            if let Err(ec) =
                validate_store_count(count, req.read_quantity.0, usize::from(req.read_quantity.0))
            {
                return encode_exception(
                    FunctionCode::ReadWriteMultipleRegisters.exception_code(),
                    ec,
                );
            }
            response
        }
        Err(ec) => encode_exception(
            FunctionCode::ReadWriteMultipleRegisters.exception_code(),
            ec,
        ),
    }
}

async fn handle_read_file_record<S: DataStore>(
    req: ReadFileRecordRequest<'_>,
    store: &S,
) -> Vec<u8> {
    let subs = req.sub_requests;
    // Each sub-request is exactly 7 bytes; the byte count must be a non-empty
    // multiple of 7 within 0x07..=0xF5 (§6.14, Figure 24 → IllegalDataValue).
    if subs.is_empty() || !subs.len().is_multiple_of(7) || subs.len() > 0xF5 {
        return encode_exception(
            FunctionCode::ReadFileRecord.exception_code(),
            ExceptionCode::IllegalDataValue,
        );
    }
    let mut response = Vec::with_capacity(MAX_PDU_SIZE);
    response.push(FunctionCode::ReadFileRecord.code());
    response.push(0);
    for chunk in subs.chunks_exact(7) {
        if chunk[0] != 6 {
            // Reference type must be 6 (Figure 24 groups this under 0x02).
            return encode_exception(
                FunctionCode::ReadFileRecord.exception_code(),
                ExceptionCode::IllegalDataAddress,
            );
        }
        let file = u16::from_be_bytes([chunk[1], chunk[2]]);
        let record = u16::from_be_bytes([chunk[3], chunk[4]]);
        let length = u16::from_be_bytes([chunk[5], chunk[6]]);
        if let Err(ec) = file_record::validate_range(file, record, usize::from(length)) {
            return encode_exception(FunctionCode::ReadFileRecord.exception_code(), ec);
        }
        let requested_count = usize::from(length);
        if requested_count > MAX_FILE_RECORD_REGISTERS {
            return encode_exception(
                FunctionCode::ReadFileRecord.exception_code(),
                ExceptionCode::IllegalDataAddress,
            );
        }
        let group_start = response.len();
        let value_byte_count = requested_count * 2;
        response.resize(group_start + 2 + value_byte_count, 0);
        match store
            .read_file_record_be(
                file,
                record,
                length,
                &mut response[group_start + 2..group_start + 2 + value_byte_count],
            )
            .await
        {
            Ok(n) => {
                let n = match validate_store_count(n, length, requested_count) {
                    Ok(n) => n,
                    Err(ec) => {
                        return encode_exception(FunctionCode::ReadFileRecord.exception_code(), ec);
                    }
                };
                // Each sub-response is [resp_len][ref_type=6][2*N data]; resp_len
                // counts the ref-type byte plus the data, excluding itself.
                let resp_len = 1 + 2 * n;
                let resp_len = match checked_response_u8(resp_len, FunctionCode::ReadFileRecord) {
                    Ok(resp_len) => resp_len,
                    Err(resp) => return resp,
                };
                response[group_start] = resp_len;
                response[group_start + 1] = 0x06;
            }
            Err(ec) => return encode_exception(FunctionCode::ReadFileRecord.exception_code(), ec),
        }
        // Response PDU is FC + byte_count(1) + data, capped at 253 bytes.
        // FC14 sub-response groups are even-sized, so the largest valid
        // byte_count is 250.
        if response.len() - 2 > 250 {
            return encode_exception(
                FunctionCode::ReadFileRecord.exception_code(),
                ExceptionCode::IllegalDataValue,
            );
        }
    }
    let byte_count = match checked_response_u8(response.len() - 2, FunctionCode::ReadFileRecord) {
        Ok(byte_count) => byte_count,
        Err(resp) => return resp,
    };
    response[1] = byte_count;
    response
}

async fn apply_write_file_record<S: DataStore>(
    req: &WriteFileRecordRequest<'_>,
    store: &S,
) -> Result<(), ExceptionCode> {
    const MAX_WRITE_FILE_RECORD_REQUEST_BYTES: usize = 0xFB;
    const MIN_WRITE_FILE_RECORD_SUB_REQUEST_BYTES: usize = 9;
    const MAX_WRITE_FILE_RECORD_GROUPS: usize =
        MAX_WRITE_FILE_RECORD_REQUEST_BYTES / MIN_WRITE_FILE_RECORD_SUB_REQUEST_BYTES;

    #[derive(Clone, Copy)]
    struct WriteFileRecordGroup<'a> {
        file: u16,
        record: u16,
        length: u16,
        value_bytes: &'a [u8],
    }

    // §6.15 field table: request data length must be 0x09..=0xFB.
    if !(0x09..=0xFB).contains(&req.byte_count) {
        return Err(ExceptionCode::IllegalDataValue);
    }
    // Validate every group before the first store call. This prevents a malformed
    // later group from causing an earlier group to be submitted.
    let mut subs = req.sub_requests;
    let mut groups = [WriteFileRecordGroup {
        file: 0,
        record: 0,
        length: 0,
        value_bytes: &[],
    }; MAX_WRITE_FILE_RECORD_GROUPS];
    let mut group_count = 0;
    while !subs.is_empty() {
        // Sub-request header [ref_type][file Hi/Lo][record Hi/Lo][len Hi/Lo]
        // followed by len*2 data bytes (§6.15).
        if subs.len() < 7 {
            return Err(ExceptionCode::IllegalDataValue);
        }
        if subs[0] != 6 {
            return Err(ExceptionCode::IllegalDataAddress);
        }
        let file = u16::from_be_bytes([subs[1], subs[2]]);
        let record = u16::from_be_bytes([subs[3], subs[4]]);
        let length = u16::from_be_bytes([subs[5], subs[6]]);
        let data_end = 7 + 2 * usize::from(length);
        if subs.len() < data_end {
            return Err(ExceptionCode::IllegalDataValue);
        }
        file_record::validate_range(file, record, usize::from(length))?;
        if group_count == groups.len() {
            return Err(ExceptionCode::IllegalDataValue);
        }
        groups[group_count] = WriteFileRecordGroup {
            file,
            record,
            length,
            value_bytes: &subs[7..data_end],
        };
        group_count += 1;
        subs = &subs[data_end..];
    }
    // Pass 2: apply the validated sub-requests.
    for group in &groups[..group_count] {
        store
            .write_file_record_be(group.file, group.record, group.length, group.value_bytes)
            .await?;
    }
    Ok(())
}

async fn handle_read_fifo_queue<S: DataStore>(address: Address, store: &S) -> Vec<u8> {
    const MAX_FIFO_VALUE_BYTES: usize = MAX_FIFO_VALUES as usize * 2;

    let mut response = vec![0u8; 5 + MAX_FIFO_VALUE_BYTES];
    response[0] = FunctionCode::ReadFifoQueue.code();

    match store
        .read_fifo_queue_be(address.0, &mut response[5..])
        .await
    {
        Ok(count) => {
            // §6.18 / Figure 28: at most 31 values. The store call ran first, so
            // an unknown address still wins as 0x02 over this 0x03.
            if count > usize::from(MAX_FIFO_VALUES) {
                return encode_exception(
                    FunctionCode::ReadFifoQueue.exception_code(),
                    ExceptionCode::IllegalDataValue,
                );
            }
            let byte_count = match checked_response_u16(2 + count * 2, FunctionCode::ReadFifoQueue)
            {
                Ok(byte_count) => byte_count,
                Err(resp) => return resp,
            };
            let fifo_count = match checked_response_u16(count, FunctionCode::ReadFifoQueue) {
                Ok(fifo_count) => fifo_count,
                Err(resp) => return resp,
            };
            response[1..3].copy_from_slice(&byte_count.to_be_bytes());
            response[3..5].copy_from_slice(&fifo_count.to_be_bytes());
            response.truncate(5 + count * 2);
            response
        }
        Err(ec) => encode_exception(FunctionCode::ReadFifoQueue.exception_code(), ec),
    }
}

fn checked_response_u8(value: usize, fc: FunctionCode) -> Result<u8, Vec<u8>> {
    u8::try_from(value)
        .map_err(|_| encode_exception(fc.exception_code(), ExceptionCode::ServerDeviceFailure))
}

fn checked_response_u16(value: usize, fc: FunctionCode) -> Result<u16, Vec<u8>> {
    u16::try_from(value)
        .map_err(|_| encode_exception(fc.exception_code(), ExceptionCode::ServerDeviceFailure))
}

fn validate_store_count(
    count: usize,
    requested: u16,
    capacity: usize,
) -> Result<usize, ExceptionCode> {
    if count == usize::from(requested) && count <= capacity {
        Ok(count)
    } else {
        Err(ExceptionCode::ServerDeviceFailure)
    }
}

fn encode_exception(fc_with_flag: u8, ec: ExceptionCode) -> Vec<u8> {
    debug!(
        function_code = fc_with_flag & 0x7F,
        exception_function_code = fc_with_flag,
        exception_code = ec.code(),
        exception = ?ec,
        "encoding Modbus exception response"
    );
    vec![fc_with_flag, ec.code()]
}
