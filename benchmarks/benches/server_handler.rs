//! Server request-dispatch microbenchmarks.

use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use rusty_modbus_server::handler::process_request;
use rusty_modbus_server::{
    CommEventLogMeta, DataStore, DeviceIdentification, InMemoryStore, StoreConfig,
};
use rusty_modbus_types::{ExceptionCode, UnitId};
use tokio::runtime::Runtime;

const UNIT: UnitId = UnitId(1);

fn bench_server_process_request(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = Arc::new(seed_store());
    let device_id = DeviceIdentification::default();

    let fc01 = [0x01, 0x00, 0x00, 0x07, 0xB0]; // 1968 coils
    let fc02 = [0x02, 0x00, 0x00, 0x07, 0xD0]; // 2000 discrete inputs
    let fc03 = [0x03, 0x00, 0x00, 0x00, 0x7D]; // 125 holding registers
    let fc0f = write_multiple_coils_pdu(0, 1968);
    let fc10 = write_multiple_registers_pdu(0, 123);
    let fc14 = [
        0x14, 0x0E, // byte count
        0x06, 0x00, 0x04, 0x00, 0x01, 0x00, 0x02, // file 4, record 1, len 2
        0x06, 0x00, 0x03, 0x00, 0x09, 0x00, 0x02, // file 3, record 9, len 2
    ];
    let fc15 = [
        0x15, 0x16, // byte count
        0x06, 0x00, 0x04, 0x00, 0x07, 0x00, 0x03, // file 4, record 7, len 3
        0x06, 0xAF, 0x04, 0xBE, 0x10, 0x0D, // group 1 data
        0x06, 0x00, 0x03, 0x00, 0x0B, 0x00, 0x01, // file 3, record 11, len 1
        0x12, 0x34, // group 2 data
    ];
    let fc17 = read_write_multiple_registers_pdu(0, 125, 0, 121);
    let fc18 = [0x18, 0x04, 0xDE];
    let fc08 = [0x08, 0x00, 0x00, 0x12, 0x34]; // Return Query Data.
    let fc0c = [0x0C]; // Get Comm Event Log.
    let fc11 = [0x11]; // Report Server ID.
    let fc2b = [0x2B, 0x0E, 0x01, 0x00]; // Read Device Identification, basic stream.
    let event_log_store = EventLogBenchStore;

    let mut group = c.benchmark_group("server_process_request");
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc01_read_coils_max",
        &fc01,
    );
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc02_read_discrete_inputs_max",
        &fc02,
    );
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc03_read_holding_registers_max",
        &fc03,
    );
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc0f_write_coils_max",
        &fc0f,
    );
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc10_write_registers_max",
        &fc10,
    );
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc14_read_file_two_groups",
        &fc14,
    );
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc15_write_file_two_groups",
        &fc15,
    );
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc17_read_write_registers_max",
        &fc17,
    );
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc18_read_fifo_two_values",
        &fc18,
    );
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc08_return_query_data",
        &fc08,
    );
    bench_pdu(
        &mut group,
        &rt,
        &event_log_store,
        &device_id,
        "fc0c_get_comm_event_log",
        &fc0c,
    );
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc11_report_server_id",
        &fc11,
    );
    bench_pdu(
        &mut group,
        &rt,
        store.as_ref(),
        &device_id,
        "fc2b_device_id_basic",
        &fc2b,
    );
    group.finish();
}

fn bench_pdu(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    rt: &Runtime,
    store: &impl DataStore,
    device_id: &DeviceIdentification,
    name: &'static str,
    pdu: &[u8],
) {
    group.bench_with_input(BenchmarkId::from_parameter(name), pdu, |b, pdu| {
        b.to_async(rt).iter(|| async {
            let response = process_request(black_box(pdu), UNIT, store, device_id)
                .await
                .expect("unit-addressed benchmark request should produce a response");
            black_box(response);
        });
    });
}

fn seed_store() -> InMemoryStore {
    let store = InMemoryStore::new(StoreConfig::default());
    for address in 0..125 {
        store
            .set_holding_register(address, address)
            .expect("benchmark register seed should fit configured table");
    }
    for address in 0..1968 {
        if address % 3 == 0 {
            store
                .set_coil(address, true)
                .expect("benchmark coil seed should fit configured table");
        }
    }
    for address in 0..2000 {
        if address % 5 == 0 {
            store
                .set_discrete_input(address, true)
                .expect("benchmark discrete input seed should fit configured table");
        }
    }
    store.set_file_record(4, 1, 0x0DFE).unwrap();
    store.set_file_record(4, 2, 0x0020).unwrap();
    store.set_file_record(3, 9, 0x33CD).unwrap();
    store.set_file_record(3, 10, 0x0040).unwrap();
    store.set_fifo_queue(0x04DE, vec![0x01B8, 0x1284]);
    store
}

struct EventLogBenchStore;

impl DataStore for EventLogBenchStore {
    async fn read_coils(&self, _: u16, _: u16, _: &mut [bool]) -> Result<usize, ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }

    async fn write_coil(&self, _: u16, _: bool) -> Result<(), ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }

    async fn write_coils(&self, _: u16, _: &[bool]) -> Result<(), ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }

    async fn read_discrete_inputs(
        &self,
        _: u16,
        _: u16,
        _: &mut [bool],
    ) -> Result<usize, ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }

    async fn read_holding_registers(
        &self,
        _: u16,
        _: u16,
        _: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }

    async fn write_register(&self, _: u16, _: u16) -> Result<(), ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }

    async fn write_registers(&self, _: u16, _: &[u16]) -> Result<(), ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }

    async fn read_input_registers(
        &self,
        _: u16,
        _: u16,
        _: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }

    async fn append_comm_event_log(
        &self,
        out: &mut Vec<u8>,
    ) -> Result<CommEventLogMeta, ExceptionCode> {
        out.extend_from_slice(&[0x20, 0x00]);
        Ok(CommEventLogMeta {
            status: 0x0000,
            event_count: 0x0108,
            message_count: 0x0121,
        })
    }
}

#[allow(clippy::cast_possible_truncation)]
fn write_multiple_registers_pdu(address: u16, register_count: usize) -> Vec<u8> {
    let mut pdu = Vec::with_capacity(6 + register_count * 2);
    pdu.push(0x10);
    pdu.extend_from_slice(&address.to_be_bytes());
    pdu.extend_from_slice(&(register_count as u16).to_be_bytes());
    pdu.push((register_count * 2) as u8);
    append_register_values(&mut pdu, register_count);
    pdu
}

#[allow(clippy::cast_possible_truncation)]
fn read_write_multiple_registers_pdu(
    read_address: u16,
    read_count: usize,
    write_address: u16,
    write_count: usize,
) -> Vec<u8> {
    let mut pdu = Vec::with_capacity(10 + write_count * 2);
    pdu.push(0x17);
    pdu.extend_from_slice(&read_address.to_be_bytes());
    pdu.extend_from_slice(&(read_count as u16).to_be_bytes());
    pdu.extend_from_slice(&write_address.to_be_bytes());
    pdu.extend_from_slice(&(write_count as u16).to_be_bytes());
    pdu.push((write_count * 2) as u8);
    append_register_values(&mut pdu, write_count);
    pdu
}

#[allow(clippy::cast_possible_truncation)]
fn write_multiple_coils_pdu(address: u16, coil_count: usize) -> Vec<u8> {
    let byte_count = coil_count.div_ceil(8);
    let mut pdu = Vec::with_capacity(6 + byte_count);
    pdu.push(0x0F);
    pdu.extend_from_slice(&address.to_be_bytes());
    pdu.extend_from_slice(&(coil_count as u16).to_be_bytes());
    pdu.push(byte_count as u8);
    for byte_index in 0..byte_count {
        let start = byte_index * 8;
        let bit_count = (coil_count - start).min(8);
        let mut byte = 0u8;
        for bit in 0..bit_count {
            if (start + bit) % 3 == 0 {
                byte |= 1 << bit;
            }
        }
        pdu.push(byte);
    }
    pdu
}

#[allow(clippy::cast_possible_truncation)]
fn append_register_values(pdu: &mut Vec<u8>, register_count: usize) {
    for value in 0..register_count {
        pdu.extend_from_slice(&(value as u16).to_be_bytes());
    }
}

criterion_group!(benches, bench_server_process_request);
criterion_main!(benches);
