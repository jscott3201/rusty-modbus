//! Request dispatch and response building.

use rusty_modbus_codec::request::{Encode, ReadFileRecordRequest, WriteFileRecordRequest};
use rusty_modbus_codec::response::{
    DiagnosticsResponse, GetCommEventCounterResponse, GetCommEventLogResponse,
    MaskWriteRegisterResponse, ReadCoilsResponse, ReadDiscreteInputsResponse,
    ReadExceptionStatusResponse, ReadFifoQueueResponse, ReadFileRecordResponse,
    ReadHoldingRegistersResponse, ReadInputRegistersResponse, ReadWriteMultipleRegistersResponse,
    ReportServerIdResponse, WriteFileRecordResponse, WriteMultipleCoilsResponse,
    WriteMultipleRegistersResponse, WriteSingleCoilResponse, WriteSingleRegisterResponse,
};
use rusty_modbus_codec::{DecodeError, RequestPdu, decode_request};
use rusty_modbus_types::{
    Address, ExceptionCode, FunctionCode, MAX_READ_COILS, MAX_READ_REGISTERS, MeiType, Quantity,
    UnitId,
};

use crate::config::DeviceIdentification;
use crate::store::DataStore;

/// Process a request PDU and return a response PDU (or `None` for broadcast writes).
///
/// The `pdu` slice starts at the function code byte.
#[allow(clippy::too_many_lines)]
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
                _ => {
                    // FC is recognized (otherwise decode would return UnknownFunctionCode),
                    // but the data is malformed (truncated, bad quantity, bad byte count,
                    // invalid coil value, etc.) → IllegalDataValue per spec §4.5
                    ExceptionCode::IllegalDataValue
                }
            };
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
            let values: Vec<u16> = req
                .register_values
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            let result = store.write_registers(req.address.0, &values).await;
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
            let mut values = Vec::with_capacity(req.quantity.0 as usize);
            for i in 0..req.quantity.0 as usize {
                values.push((req.coil_values[i / 8] >> (i % 8)) & 1 == 1);
            }
            let result = store.write_coils(req.address.0, &values).await;
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
            let result = store.diagnostic(req.sub_function, req.data).await;
            if is_broadcast {
                return None;
            }
            match result {
                Ok(Some(data)) => Some(encode_response(&DiagnosticsResponse {
                    sub_function: req.sub_function,
                    data: &data,
                })),
                Ok(None) => None,
                Err(ec) => Some(encode_exception(
                    FunctionCode::Diagnostics.exception_code(),
                    ec,
                )),
            }
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
            Some(match store.get_comm_event_log().await {
                // §6.10 caps the event log at 64 bytes; a larger log would make the
                // byte_count unrepresentable / the frame malformed, so a store that
                // returns more is failing → ServerDeviceFailure.
                Ok(log) if log.events.len() > 64 => encode_exception(
                    FunctionCode::GetCommEventLog.exception_code(),
                    ExceptionCode::ServerDeviceFailure,
                ),
                Ok(log) => {
                    // Derive the wire byte_count (status+event+message = 6 bytes + events).
                    let byte_count = u8::try_from(log.events.len() + 6).unwrap_or(u8::MAX);
                    encode_response(&GetCommEventLogResponse {
                        byte_count,
                        status: log.status,
                        event_count: log.event_count,
                        message_count: log.message_count,
                        events: &log.events,
                    })
                }
                Err(ec) => encode_exception(FunctionCode::GetCommEventLog.exception_code(), ec),
            })
        }
        RequestPdu::ReportServerId => {
            if is_broadcast {
                return None;
            }
            Some(match store.report_server_id().await {
                // The PDU is FC + byte_count(u8) + data, capped at 253 bytes; a blob
                // over 251 bytes can't be represented and would panic the encoder
                // (copy_from_slice into a byte_count-sized slice) → ServerDeviceFailure.
                Ok(data) if data.len() > 251 => encode_exception(
                    FunctionCode::ReportServerId.exception_code(),
                    ExceptionCode::ServerDeviceFailure,
                ),
                Ok(data) => {
                    // byte_count MUST equal data.len() or the encoder panics.
                    let byte_count = u8::try_from(data.len()).unwrap_or(u8::MAX);
                    encode_response(&ReportServerIdResponse {
                        byte_count,
                        data: &data,
                    })
                }
                Err(ec) => encode_exception(FunctionCode::ReportServerId.exception_code(), ec),
            })
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

async fn handle_read_registers<S: DataStore>(
    fc: FunctionCode,
    address: Address,
    quantity: Quantity,
    store: &S,
    is_holding: bool,
) -> Vec<u8> {
    let mut buf = [0u16; MAX_READ_REGISTERS as usize];
    let result = if is_holding {
        store
            .read_holding_registers(address.0, quantity.0, &mut buf)
            .await
    } else {
        store
            .read_input_registers(address.0, quantity.0, &mut buf)
            .await
    };

    match result {
        Ok(count) => {
            let byte_count = u8::try_from(count * 2).unwrap_or(u8::MAX);
            let mut data = vec![0u8; count * 2];
            for (i, &val) in buf[..count].iter().enumerate() {
                data[i * 2..i * 2 + 2].copy_from_slice(&val.to_be_bytes());
            }
            if is_holding {
                encode_response(&ReadHoldingRegistersResponse {
                    byte_count,
                    register_data: &data,
                })
            } else {
                encode_response(&ReadInputRegistersResponse {
                    byte_count,
                    register_data: &data,
                })
            }
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
    let mut buf = [false; MAX_READ_COILS as usize];
    let result = if is_coils {
        store.read_coils(address.0, quantity.0, &mut buf).await
    } else {
        store
            .read_discrete_inputs(address.0, quantity.0, &mut buf)
            .await
    };

    match result {
        Ok(count) => {
            let byte_count = u8::try_from(count.div_ceil(8)).unwrap_or(u8::MAX);
            let mut bit_bytes = vec![0u8; byte_count as usize];
            for (i, &val) in buf[..count].iter().enumerate() {
                if val {
                    bit_bytes[i / 8] |= 1 << (i % 8);
                }
            }
            if is_coils {
                encode_response(&ReadCoilsResponse {
                    byte_count,
                    coil_status: &bit_bytes,
                })
            } else {
                encode_response(&ReadDiscreteInputsResponse {
                    byte_count,
                    coil_status: &bit_bytes,
                })
            }
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
    let mut buf = [0u16; 1];
    store.read_holding_registers(address.0, 1, &mut buf).await?;
    let result = (buf[0] & and_mask) | (or_mask & !and_mask);
    store.write_register(address.0, result).await
}

async fn handle_read_write_multiple<S: DataStore>(
    req: rusty_modbus_codec::request::ReadWriteMultipleRegistersRequest<'_>,
    store: &S,
) -> Vec<u8> {
    // Write executes before read per spec §6.17.
    let write_values: Vec<u16> = req
        .write_register_values
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();

    if let Err(ec) = store
        .write_registers(req.write_address.0, &write_values)
        .await
    {
        return encode_exception(
            FunctionCode::ReadWriteMultipleRegisters.exception_code(),
            ec,
        );
    }

    let mut buf = [0u16; MAX_READ_REGISTERS as usize];
    match store
        .read_holding_registers(req.read_address.0, req.read_quantity.0, &mut buf)
        .await
    {
        Ok(count) => {
            let byte_count = u8::try_from(count * 2).unwrap_or(u8::MAX);
            let mut data = vec![0u8; count * 2];
            for (i, &val) in buf[..count].iter().enumerate() {
                data[i * 2..i * 2 + 2].copy_from_slice(&val.to_be_bytes());
            }
            encode_response(&ReadWriteMultipleRegistersResponse {
                byte_count,
                register_data: &data,
            })
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
    let mut data: Vec<u8> = Vec::new();
    let mut scratch = [0u16; 122]; // 0xF5 / 2 = max registers in one sub-response
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
        match store
            .read_file_record(file, record, length, &mut scratch)
            .await
        {
            // Guard against a misbehaving store reporting more than it could write.
            Ok(n) if n > scratch.len() => {
                return encode_exception(
                    FunctionCode::ReadFileRecord.exception_code(),
                    ExceptionCode::ServerDeviceFailure,
                );
            }
            Ok(n) => {
                // Each sub-response is [resp_len][ref_type=6][2*N data]; resp_len
                // counts the ref-type byte plus the data, excluding itself.
                let resp_len = 1 + 2 * n;
                data.push(u8::try_from(resp_len).unwrap_or(u8::MAX));
                data.push(0x06);
                for &w in &scratch[..n] {
                    data.extend_from_slice(&w.to_be_bytes());
                }
            }
            Err(ec) => return encode_exception(FunctionCode::ReadFileRecord.exception_code(), ec),
        }
        // Response PDU is FC + byte_count(1) + data, capped at 253 bytes.
        if data.len() > 251 {
            return encode_exception(
                FunctionCode::ReadFileRecord.exception_code(),
                ExceptionCode::IllegalDataValue,
            );
        }
    }
    let byte_count = u8::try_from(data.len()).unwrap_or(u8::MAX);
    encode_response(&ReadFileRecordResponse {
        byte_count,
        data: &data,
    })
}

async fn apply_write_file_record<S: DataStore>(
    req: &WriteFileRecordRequest<'_>,
    store: &S,
) -> Result<(), ExceptionCode> {
    // §6.15 / Figure 25: byte_count must be 0x07..=0xF5.
    if req.byte_count > 0xF5 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    // Pass 1: validate framing and collect every sub-request *before* writing, so
    // a malformed later sub-request rejects the whole request without committing
    // the earlier ones (write is atomic with respect to framing errors).
    let mut subs = req.sub_requests;
    let mut groups: Vec<(u16, u16, Vec<u16>)> = Vec::new();
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
        let length = usize::from(u16::from_be_bytes([subs[5], subs[6]]));
        let data_end = 7 + 2 * length;
        if subs.len() < data_end {
            return Err(ExceptionCode::IllegalDataValue);
        }
        let values: Vec<u16> = subs[7..data_end]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        groups.push((file, record, values));
        subs = &subs[data_end..];
    }
    // Pass 2: apply the validated sub-requests.
    for (file, record, values) in &groups {
        store.write_file_record(*file, *record, values).await?;
    }
    Ok(())
}

async fn handle_read_fifo_queue<S: DataStore>(address: Address, store: &S) -> Vec<u8> {
    match store.read_fifo_queue(address.0).await {
        Ok(values) => {
            // §6.18 / Figure 28: at most 31 values. The store call ran first, so
            // an unknown address still wins as 0x02 over this 0x03.
            if values.len() > 31 {
                return encode_exception(
                    FunctionCode::ReadFifoQueue.exception_code(),
                    ExceptionCode::IllegalDataValue,
                );
            }
            let n = values.len();
            let mut bytes = vec![0u8; n * 2];
            for (i, &v) in values.iter().enumerate() {
                bytes[i * 2..i * 2 + 2].copy_from_slice(&v.to_be_bytes());
            }
            encode_response(&ReadFifoQueueResponse {
                byte_count: u16::try_from(2 + n * 2).unwrap_or(u16::MAX),
                fifo_count: u16::try_from(n).unwrap_or(u16::MAX),
                fifo_values: &bytes,
            })
        }
        Err(ec) => encode_exception(FunctionCode::ReadFifoQueue.exception_code(), ec),
    }
}

fn encode_response(resp: &dyn Encode) -> Vec<u8> {
    let mut buf = vec![0u8; resp.encoded_len()];
    let _ = resp.encode_into(&mut buf);
    buf
}

fn encode_exception(fc_with_flag: u8, ec: ExceptionCode) -> Vec<u8> {
    vec![fc_with_flag, ec.code()]
}

/// Build a Read Device Identification response PDU (FC 0x2B / MEI 0x0E).
fn build_device_id_response(mei_data: &[u8], device_id: &DeviceIdentification) -> Vec<u8> {
    let device_id_code = mei_data.first().copied().unwrap_or(0x01);

    let mut objects: Vec<(u8, &[u8])> = vec![
        (0x00, device_id.vendor_name.as_bytes()),
        (0x01, device_id.product_code.as_bytes()),
        (0x02, device_id.major_minor_revision.as_bytes()),
    ];

    let has_regular = device_id.vendor_url.is_some()
        || device_id.product_name.is_some()
        || device_id.model_name.is_some()
        || device_id.user_application_name.is_some();

    if let Some(ref v) = device_id.vendor_url {
        objects.push((0x03, v.as_bytes()));
    }
    if let Some(ref v) = device_id.product_name {
        objects.push((0x04, v.as_bytes()));
    }
    if let Some(ref v) = device_id.model_name {
        objects.push((0x05, v.as_bytes()));
    }
    if let Some(ref v) = device_id.user_application_name {
        objects.push((0x06, v.as_bytes()));
    }

    let conformity_level: u8 = if has_regular { 0x02 } else { 0x01 };

    #[allow(clippy::cast_possible_truncation)]
    let mut resp = vec![
        0x2B, // FC
        0x0E, // MEI type
        device_id_code,
        conformity_level,
        0x00, // more_follows = false
        0x00, // next_object_id
        objects.len() as u8,
    ];

    for (id, value) in &objects {
        resp.push(*id);
        #[allow(clippy::cast_possible_truncation)]
        resp.push(value.len() as u8);
        resp.extend_from_slice(value);
    }

    resp
}
