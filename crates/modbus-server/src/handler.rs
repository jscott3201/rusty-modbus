//! Request dispatch and response building.

use modbus_codec::request::Encode;
use modbus_codec::response::{
    MaskWriteRegisterResponse, ReadCoilsResponse, ReadDiscreteInputsResponse,
    ReadHoldingRegistersResponse, ReadInputRegistersResponse, ReadWriteMultipleRegistersResponse,
    WriteMultipleCoilsResponse, WriteMultipleRegistersResponse, WriteSingleCoilResponse,
    WriteSingleRegisterResponse,
};
use modbus_codec::{decode_request, DecodeError, RequestPdu};
use modbus_types::{
    Address, ExceptionCode, FunctionCode, MeiType, Quantity, UnitId, MAX_READ_COILS,
    MAX_READ_REGISTERS,
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
                DecodeError::UnknownFunctionCode(_) => ExceptionCode::IllegalFunction,
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
            if is_broadcast { return None; }
            Some(handle_read_registers(FunctionCode::ReadHoldingRegisters, req.address, req.quantity, store, true).await)
        }
        RequestPdu::ReadInputRegisters(req) => {
            if is_broadcast { return None; }
            Some(handle_read_registers(FunctionCode::ReadInputRegisters, req.address, req.quantity, store, false).await)
        }
        RequestPdu::ReadCoils(req) => {
            if is_broadcast { return None; }
            Some(handle_read_bits(FunctionCode::ReadCoils, req.address, req.quantity, store, true).await)
        }
        RequestPdu::ReadDiscreteInputs(req) => {
            if is_broadcast { return None; }
            Some(handle_read_bits(FunctionCode::ReadDiscreteInputs, req.address, req.quantity, store, false).await)
        }
        RequestPdu::WriteSingleRegister(req) => {
            let result = store.write_register(req.address.0, req.value).await;
            if is_broadcast { return None; }
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
            if is_broadcast { return None; }
            Some(match result {
                Ok(()) => encode_response(&WriteMultipleRegistersResponse {
                    address: req.address,
                    quantity: req.quantity,
                }),
                Err(ec) => encode_exception(FunctionCode::WriteMultipleRegisters.exception_code(), ec),
            })
        }
        RequestPdu::WriteSingleCoil(req) => {
            let result = store.write_coil(req.address.0, req.value.as_bool()).await;
            if is_broadcast { return None; }
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
            if is_broadcast { return None; }
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
            if is_broadcast { return None; }
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
            if is_broadcast { return None; }
            Some(handle_read_write_multiple(req, store).await)
        }
        RequestPdu::EncapsulatedInterface(req) => {
            if is_broadcast { return None; }
            if req.mei_type == MeiType::ReadDeviceIdentification {
                Some(build_device_id_response(req.data, device_id))
            } else {
                let fc = pdu.first().copied().unwrap_or(0);
                Some(encode_exception(fc | 0x80, ExceptionCode::IllegalFunction))
            }
        }
        _ => {
            if is_broadcast { return None; }
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
        store.read_holding_registers(address.0, quantity.0, &mut buf).await
    } else {
        store.read_input_registers(address.0, quantity.0, &mut buf).await
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
        store.read_discrete_inputs(address.0, quantity.0, &mut buf).await
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
    req: modbus_codec::request::ReadWriteMultipleRegistersRequest<'_>,
    store: &S,
) -> Vec<u8> {
    // Write executes before read per spec §6.17.
    let write_values: Vec<u16> = req
        .write_register_values
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();

    if let Err(ec) = store.write_registers(req.write_address.0, &write_values).await {
        return encode_exception(
            FunctionCode::ReadWriteMultipleRegisters.exception_code(),
            ec,
        );
    }

    let mut buf = [0u16; MAX_READ_REGISTERS as usize];
    match store.read_holding_registers(req.read_address.0, req.read_quantity.0, &mut buf).await {
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
