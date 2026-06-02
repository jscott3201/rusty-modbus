//! Server-side Read Device Identification response building.

use rusty_modbus_types::{DeviceIdCode, ExceptionCode, FunctionCode, MAX_PDU_SIZE};

use crate::config::DeviceIdentification;

const DEVICE_ID_RESPONSE_HEADER_LEN: usize = 7;
const DEVICE_ID_OBJECT_HEADER_LEN: usize = 2;
const DEVICE_ID_REQUEST_LEN: usize = 2;
const MAX_DEVICE_ID_OBJECT_VALUE_LEN: usize =
    MAX_PDU_SIZE - DEVICE_ID_RESPONSE_HEADER_LEN - DEVICE_ID_OBJECT_HEADER_LEN;

#[derive(Clone, Copy)]
struct DeviceIdObject<'a> {
    id: u8,
    value: &'a [u8],
}

/// Build a Read Device Identification response PDU (FC 0x2B / MEI 0x0E).
pub(crate) fn build_device_id_response(
    mei_data: &[u8],
    device_id: &DeviceIdentification,
) -> Vec<u8> {
    if mei_data.len() != DEVICE_ID_REQUEST_LEN {
        return device_id_exception(ExceptionCode::IllegalDataValue);
    }
    let Some((&raw_device_id_code, rest)) = mei_data.split_first() else {
        return device_id_exception(ExceptionCode::IllegalDataValue);
    };
    let Some(&object_id) = rest.first() else {
        return device_id_exception(ExceptionCode::IllegalDataValue);
    };
    let Some(device_id_code) = DeviceIdCode::from_raw(raw_device_id_code) else {
        return device_id_exception(ExceptionCode::IllegalDataValue);
    };

    let objects = configured_objects(device_id);
    let conformity_level = conformity_level(device_id);

    match device_id_code {
        DeviceIdCode::Individual => {
            let Some(object) = objects
                .iter()
                .find(|object| object.id == object_id)
                .copied()
            else {
                return device_id_exception(ExceptionCode::IllegalDataAddress);
            };
            if object.value.len() > MAX_DEVICE_ID_OBJECT_VALUE_LEN {
                return device_id_exception(ExceptionCode::ServerDeviceFailure);
            }
            build_response(device_id_code, conformity_level, false, 0, &[object])
        }
        DeviceIdCode::BasicStream => {
            let stream_objects: Vec<_> = objects
                .iter()
                .copied()
                .filter(|object| object.id <= 0x02)
                .collect();
            build_stream_response(device_id_code, conformity_level, object_id, &stream_objects)
        }
        DeviceIdCode::RegularStream | DeviceIdCode::ExtendedStream => {
            build_stream_response(device_id_code, conformity_level, object_id, &objects)
        }
    }
}

fn configured_objects(device_id: &DeviceIdentification) -> Vec<DeviceIdObject<'_>> {
    let mut objects = vec![
        DeviceIdObject {
            id: 0x00,
            value: device_id.vendor_name.as_bytes(),
        },
        DeviceIdObject {
            id: 0x01,
            value: device_id.product_code.as_bytes(),
        },
        DeviceIdObject {
            id: 0x02,
            value: device_id.major_minor_revision.as_bytes(),
        },
    ];

    if let Some(ref value) = device_id.vendor_url {
        objects.push(DeviceIdObject {
            id: 0x03,
            value: value.as_bytes(),
        });
    }
    if let Some(ref value) = device_id.product_name {
        objects.push(DeviceIdObject {
            id: 0x04,
            value: value.as_bytes(),
        });
    }
    if let Some(ref value) = device_id.model_name {
        objects.push(DeviceIdObject {
            id: 0x05,
            value: value.as_bytes(),
        });
    }
    if let Some(ref value) = device_id.user_application_name {
        objects.push(DeviceIdObject {
            id: 0x06,
            value: value.as_bytes(),
        });
    }

    objects
}

fn conformity_level(device_id: &DeviceIdentification) -> u8 {
    if device_id.vendor_url.is_some()
        || device_id.product_name.is_some()
        || device_id.model_name.is_some()
        || device_id.user_application_name.is_some()
    {
        0x82
    } else {
        0x81
    }
}

fn build_stream_response(
    device_id_code: DeviceIdCode,
    conformity_level: u8,
    object_id: u8,
    objects: &[DeviceIdObject<'_>],
) -> Vec<u8> {
    if objects
        .iter()
        .any(|object| object.value.len() > MAX_DEVICE_ID_OBJECT_VALUE_LEN)
    {
        return device_id_exception(ExceptionCode::ServerDeviceFailure);
    }

    let start = objects
        .iter()
        .position(|object| object.id == object_id)
        .unwrap_or(0);

    let mut response_len = DEVICE_ID_RESPONSE_HEADER_LEN;
    let mut selected = Vec::new();
    let mut more_follows = false;
    let mut next_object_id = 0;

    for object in &objects[start..] {
        let object_len = DEVICE_ID_OBJECT_HEADER_LEN + object.value.len();
        if response_len + object_len > MAX_PDU_SIZE {
            more_follows = true;
            next_object_id = object.id;
            break;
        }
        selected.push(*object);
        response_len += object_len;
    }

    if selected.is_empty() {
        return device_id_exception(ExceptionCode::ServerDeviceFailure);
    }

    build_response(
        device_id_code,
        conformity_level,
        more_follows,
        next_object_id,
        &selected,
    )
}

#[allow(clippy::cast_possible_truncation)]
fn build_response(
    device_id_code: DeviceIdCode,
    conformity_level: u8,
    more_follows: bool,
    next_object_id: u8,
    objects: &[DeviceIdObject<'_>],
) -> Vec<u8> {
    let mut response = vec![
        FunctionCode::EncapsulatedInterfaceTransport.code(),
        0x0E,
        device_id_code.code(),
        conformity_level,
        if more_follows { 0xFF } else { 0x00 },
        if more_follows { next_object_id } else { 0x00 },
        objects.len() as u8,
    ];

    for object in objects {
        response.push(object.id);
        response.push(object.value.len() as u8);
        response.extend_from_slice(object.value);
    }

    response
}

fn device_id_exception(exception: ExceptionCode) -> Vec<u8> {
    vec![
        FunctionCode::EncapsulatedInterfaceTransport.exception_code(),
        exception.code(),
    ]
}
