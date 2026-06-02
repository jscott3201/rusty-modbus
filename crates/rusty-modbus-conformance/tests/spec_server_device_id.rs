//! Server-path conformance for FC 0x2B / MEI 0x0E Read Device Identification.

use rusty_modbus_codec::response::device_id::ReadDeviceIdentificationResponse;
use rusty_modbus_server::handler::process_request;
use rusty_modbus_server::{DeviceIdentification, InMemoryStore, StoreConfig};
use rusty_modbus_types::{DeviceIdCode, UnitId};

const UNIT: UnitId = UnitId(1);

fn store() -> InMemoryStore {
    InMemoryStore::new(StoreConfig {
        coil_count: 1,
        discrete_input_count: 1,
        holding_register_count: 1,
        input_register_count: 1,
    })
}

async fn respond(device_id: &DeviceIdentification, pdu: &[u8]) -> Vec<u8> {
    process_request(pdu, UNIT, &store(), device_id)
        .await
        .expect("a non-broadcast request must produce a response")
}

fn decode_device_id_response(response: &[u8]) -> ReadDeviceIdentificationResponse<'_> {
    assert_eq!(response[0], 0x2B);
    assert!(response.len() <= 253, "response PDU exceeded Modbus cap");
    ReadDeviceIdentificationResponse::decode(&response[1..]).unwrap()
}

#[tokio::test]
async fn device_id_basic_stream_paginates_between_objects() {
    let device_id = DeviceIdentification {
        vendor_name: "V".repeat(120),
        product_code: "P".repeat(120),
        major_minor_revision: "R".repeat(10),
        ..DeviceIdentification::default()
    };

    let first = respond(&device_id, &[0x2B, 0x0E, 0x01, 0x00]).await;
    let first = decode_device_id_response(&first);
    assert_eq!(first.device_id_code, DeviceIdCode::BasicStream);
    assert_eq!(first.conformity_level, 0x81);
    assert!(first.more_follows);
    assert_eq!(first.next_object_id, 0x02);
    assert_eq!(first.num_objects, 2);

    let first_objects: Vec<_> = first.objects().collect();
    assert_eq!(first_objects[0].id, 0x00);
    assert_eq!(first_objects[0].value.len(), 120);
    assert_eq!(first_objects[1].id, 0x01);
    assert_eq!(first_objects[1].value.len(), 120);

    let second = respond(&device_id, &[0x2B, 0x0E, 0x01, 0x02]).await;
    let second = decode_device_id_response(&second);
    assert!(!second.more_follows);
    assert_eq!(second.next_object_id, 0x00);
    assert_eq!(second.num_objects, 1);

    let second_objects: Vec<_> = second.objects().collect();
    assert_eq!(second_objects[0].id, 0x02);
    assert_eq!(second_objects[0].value.len(), 10);
}

#[tokio::test]
async fn device_id_individual_access_returns_one_configured_object() {
    let device_id = DeviceIdentification {
        product_name: Some(String::from("Pump Controller")),
        ..DeviceIdentification::default()
    };

    let response = respond(&device_id, &[0x2B, 0x0E, 0x04, 0x04]).await;
    let response = decode_device_id_response(&response);
    assert_eq!(response.device_id_code, DeviceIdCode::Individual);
    assert_eq!(response.conformity_level, 0x82);
    assert!(!response.more_follows);
    assert_eq!(response.num_objects, 1);

    let objects: Vec<_> = response.objects().collect();
    assert_eq!(objects[0].id, 0x04);
    assert_eq!(objects[0].value, b"Pump Controller");
}

#[tokio::test]
async fn device_id_stream_unknown_object_restarts_at_zero() {
    let response = respond(&DeviceIdentification::default(), &[0x2B, 0x0E, 0x01, 0x77]).await;
    let response = decode_device_id_response(&response);
    assert!(!response.more_follows);
    assert_eq!(response.num_objects, 3);

    let ids: Vec<_> = response.objects().map(|object| object.id).collect();
    assert_eq!(ids, vec![0x00, 0x01, 0x02]);
}

#[tokio::test]
async fn device_id_individual_unknown_object_is_illegal_data_address() {
    assert_eq!(
        respond(&DeviceIdentification::default(), &[0x2B, 0x0E, 0x04, 0x7F]).await,
        vec![0xAB, 0x02]
    );
}

#[tokio::test]
async fn device_id_invalid_code_is_illegal_data_value() {
    assert_eq!(
        respond(&DeviceIdentification::default(), &[0x2B, 0x0E, 0x05, 0x00]).await,
        vec![0xAB, 0x03]
    );
}

#[tokio::test]
async fn device_id_truncated_payload_is_illegal_data_value() {
    assert_eq!(
        respond(&DeviceIdentification::default(), &[0x2B, 0x0E, 0x01]).await,
        vec![0xAB, 0x03]
    );
}

#[tokio::test]
async fn device_id_oversized_indivisible_object_is_server_device_failure() {
    let device_id = DeviceIdentification {
        vendor_name: "V".repeat(245),
        ..DeviceIdentification::default()
    };

    assert_eq!(
        respond(&device_id, &[0x2B, 0x0E, 0x01, 0x00]).await,
        vec![0xAB, 0x04]
    );
}
