//! Server read paths reject misbehaving `DataStore` count reports.
//!
//! A normal Modbus read response cannot be partial: the response byte count is
//! implied by the requested quantity. These tests use a store that reports the
//! wrong number of written items so the handler must return ServerDeviceFailure
//! instead of emitting a malformed normal response or panicking.

use rusty_modbus_server::handler::process_request;
use rusty_modbus_server::{DataStore, DeviceIdentification};
use rusty_modbus_types::{DiagnosticSubFunction, ExceptionCode, UnitId};

const UNIT: UnitId = UnitId(1);

struct BadCountStore {
    count: usize,
}

impl BadCountStore {
    fn new(count: usize) -> Self {
        Self { count }
    }
}

impl DataStore for BadCountStore {
    async fn read_coils(&self, _: u16, _: u16, buf: &mut [bool]) -> Result<usize, ExceptionCode> {
        let limit = self.count.min(buf.len());
        for slot in buf.iter_mut().take(limit) {
            *slot = true;
        }
        Ok(self.count)
    }

    async fn write_coil(&self, _: u16, _: bool) -> Result<(), ExceptionCode> {
        Ok(())
    }

    async fn write_coils(&self, _: u16, _: &[bool]) -> Result<(), ExceptionCode> {
        Ok(())
    }

    async fn read_discrete_inputs(
        &self,
        _: u16,
        _: u16,
        buf: &mut [bool],
    ) -> Result<usize, ExceptionCode> {
        let limit = self.count.min(buf.len());
        for slot in buf.iter_mut().take(limit) {
            *slot = true;
        }
        Ok(self.count)
    }

    async fn read_holding_registers(
        &self,
        _: u16,
        _: u16,
        buf: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        let limit = self.count.min(buf.len());
        for slot in buf.iter_mut().take(limit) {
            *slot = 0x1234;
        }
        Ok(self.count)
    }

    async fn write_register(&self, _: u16, _: u16) -> Result<(), ExceptionCode> {
        Ok(())
    }

    async fn write_registers(&self, _: u16, _: &[u16]) -> Result<(), ExceptionCode> {
        Ok(())
    }

    async fn read_input_registers(
        &self,
        _: u16,
        _: u16,
        buf: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        let limit = self.count.min(buf.len());
        for slot in buf.iter_mut().take(limit) {
            *slot = 0x5678;
        }
        Ok(self.count)
    }

    async fn read_file_record(
        &self,
        _: u16,
        _: u16,
        _: u16,
        buf: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        let limit = self.count.min(buf.len());
        for slot in buf.iter_mut().take(limit) {
            *slot = 0x9ABC;
        }
        Ok(self.count)
    }

    async fn diagnostic(
        &self,
        _: DiagnosticSubFunction,
        _: &[u8],
    ) -> Result<Option<Vec<u8>>, ExceptionCode> {
        Err(ExceptionCode::IllegalFunction)
    }
}

async fn respond(store: &BadCountStore, pdu: &[u8]) -> Vec<u8> {
    process_request(pdu, UNIT, store, &DeviceIdentification::default())
        .await
        .expect("non-broadcast request should respond")
}

#[tokio::test]
async fn register_read_partial_count_is_server_device_failure() {
    assert_eq!(
        respond(&BadCountStore::new(1), &[0x03, 0x00, 0x00, 0x00, 0x02]).await,
        vec![0x83, 0x04]
    );
}

#[tokio::test]
async fn input_register_read_partial_count_is_server_device_failure() {
    assert_eq!(
        respond(&BadCountStore::new(1), &[0x04, 0x00, 0x00, 0x00, 0x02]).await,
        vec![0x84, 0x04]
    );
}

#[tokio::test]
async fn register_read_overreported_count_is_server_device_failure() {
    assert_eq!(
        respond(&BadCountStore::new(126), &[0x03, 0x00, 0x00, 0x00, 0x02]).await,
        vec![0x83, 0x04]
    );
}

#[tokio::test]
async fn register_read_small_overreported_count_is_server_device_failure() {
    assert_eq!(
        respond(&BadCountStore::new(3), &[0x03, 0x00, 0x00, 0x00, 0x02]).await,
        vec![0x83, 0x04]
    );
}

#[tokio::test]
async fn input_register_read_small_overreported_count_is_server_device_failure() {
    assert_eq!(
        respond(&BadCountStore::new(3), &[0x04, 0x00, 0x00, 0x00, 0x02]).await,
        vec![0x84, 0x04]
    );
}

#[tokio::test]
async fn coil_read_partial_count_is_server_device_failure() {
    assert_eq!(
        respond(&BadCountStore::new(1), &[0x01, 0x00, 0x00, 0x00, 0x08]).await,
        vec![0x81, 0x04]
    );
}

#[tokio::test]
async fn coil_read_overreported_count_is_server_device_failure() {
    assert_eq!(
        respond(&BadCountStore::new(2001), &[0x01, 0x00, 0x00, 0x00, 0x08]).await,
        vec![0x81, 0x04]
    );
}

#[tokio::test]
async fn coil_read_small_overreported_count_is_server_device_failure() {
    assert_eq!(
        respond(&BadCountStore::new(9), &[0x01, 0x00, 0x00, 0x00, 0x08]).await,
        vec![0x81, 0x04]
    );
}

#[tokio::test]
async fn discrete_input_read_partial_count_is_server_device_failure() {
    assert_eq!(
        respond(&BadCountStore::new(1), &[0x02, 0x00, 0x00, 0x00, 0x08]).await,
        vec![0x82, 0x04]
    );
}

#[tokio::test]
async fn discrete_input_read_small_overreported_count_is_server_device_failure() {
    assert_eq!(
        respond(&BadCountStore::new(9), &[0x02, 0x00, 0x00, 0x00, 0x08]).await,
        vec![0x82, 0x04]
    );
}

#[tokio::test]
async fn read_write_multiple_partial_read_count_is_server_device_failure() {
    let req = [
        0x17, // FC17
        0x00, 0x00, // read address
        0x00, 0x02, // read quantity
        0x00, 0x00, // write address
        0x00, 0x01, // write quantity
        0x02, // write byte count
        0xAA, 0xAA,
    ];
    assert_eq!(
        respond(&BadCountStore::new(1), &req).await,
        vec![0x97, 0x04]
    );
}

#[tokio::test]
async fn read_write_multiple_overreported_read_count_is_server_device_failure() {
    let req = [
        0x17, // FC17
        0x00, 0x00, // read address
        0x00, 0x02, // read quantity
        0x00, 0x00, // write address
        0x00, 0x01, // write quantity
        0x02, // write byte count
        0xAA, 0xAA,
    ];
    assert_eq!(
        respond(&BadCountStore::new(3), &req).await,
        vec![0x97, 0x04]
    );
}

#[tokio::test]
async fn file_record_partial_count_is_server_device_failure() {
    let req = [0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02];
    assert_eq!(
        respond(&BadCountStore::new(1), &req).await,
        vec![0x94, 0x04]
    );
}
