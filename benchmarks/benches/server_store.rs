//! In-memory server store write-path benchmarks.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use rusty_modbus_server::{DataStore, InMemoryStore, StoreConfig};
use rusty_modbus_types::{MAX_FIFO_VALUES, MAX_READ_COILS, MAX_READ_REGISTERS};
use tokio::runtime::Runtime;

const MAX_READ_COIL_COUNT: usize = MAX_READ_COILS as usize;
const MAX_READ_COIL_BYTES: usize = MAX_READ_COIL_COUNT.div_ceil(8);
const MAX_READ_REGISTER_COUNT: usize = MAX_READ_REGISTERS as usize;
const MAX_READ_REGISTER_BYTES: usize = MAX_READ_REGISTER_COUNT * 2;
const MAX_FIFO_VALUE_COUNT: usize = MAX_FIFO_VALUES as usize;
const MAX_FIFO_VALUE_BYTES: usize = MAX_FIFO_VALUE_COUNT * 2;

fn bench_in_memory_store_register_writes(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = InMemoryStore::new(StoreConfig::default());
    let values: Vec<u16> = (0..123).collect();
    let value_bytes = register_value_bytes(123);

    let mut group = c.benchmark_group("in_memory_store_register_writes");
    group.bench_with_input(
        BenchmarkId::from_parameter("slice_u16_max"),
        &values[..],
        |b, values| {
            b.to_async(&rt).iter(|| async {
                store.write_registers(0, black_box(values)).await.unwrap();
            });
        },
    );
    group.bench_with_input(
        BenchmarkId::from_parameter("wire_be_max"),
        &value_bytes[..],
        |b, value_bytes| {
            b.to_async(&rt).iter(|| async {
                store
                    .write_registers_be(0, 123, black_box(value_bytes))
                    .await
                    .unwrap();
            });
        },
    );
    group.bench_with_input(
        BenchmarkId::from_parameter("wire_be_via_vec_u16_max"),
        &value_bytes[..],
        |b, value_bytes| {
            b.to_async(&rt).iter(|| async {
                let values: Vec<u16> = black_box(value_bytes)
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                store.write_registers(0, black_box(&values)).await.unwrap();
            });
        },
    );
    group.finish();
}

fn bench_in_memory_store_register_reads(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = InMemoryStore::new(StoreConfig::default());
    seed_holding_registers(&store, MAX_READ_REGISTER_COUNT);

    let mut group = c.benchmark_group("in_memory_store_register_reads");
    group.bench_function("wire_be_max", |b| {
        b.to_async(&rt).iter(|| async {
            let mut bytes = [0u8; MAX_READ_REGISTER_BYTES];
            store
                .read_holding_registers_be(0, MAX_READ_REGISTERS, black_box(&mut bytes))
                .await
                .unwrap();
            black_box(bytes);
        });
    });
    group.bench_function("slice_u16_then_pack_max", |b| {
        b.to_async(&rt).iter(|| async {
            let mut values = [0u16; MAX_READ_REGISTER_COUNT];
            let mut bytes = [0u8; MAX_READ_REGISTER_BYTES];
            store
                .read_holding_registers(0, MAX_READ_REGISTERS, black_box(&mut values))
                .await
                .unwrap();
            pack_register_slice(black_box(&values), &mut bytes);
            black_box(bytes);
        });
    });
    group.finish();
}

fn bench_in_memory_store_coil_writes(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = InMemoryStore::new(StoreConfig::default());
    let values: Vec<bool> = (0..1968).map(|i| i % 3 == 0).collect();
    let packed_values = packed_coil_values(1968);

    let mut group = c.benchmark_group("in_memory_store_coil_writes");
    group.bench_with_input(
        BenchmarkId::from_parameter("slice_bool_max"),
        &values[..],
        |b, values| {
            b.to_async(&rt).iter(|| async {
                store.write_coils(0, black_box(values)).await.unwrap();
            });
        },
    );
    group.bench_with_input(
        BenchmarkId::from_parameter("wire_packed_max"),
        &packed_values[..],
        |b, packed_values| {
            b.to_async(&rt).iter(|| async {
                store
                    .write_coils_packed(0, 1968, black_box(packed_values))
                    .await
                    .unwrap();
            });
        },
    );
    group.bench_with_input(
        BenchmarkId::from_parameter("wire_packed_via_vec_bool_max"),
        &packed_values[..],
        |b, packed_values| {
            b.to_async(&rt).iter(|| async {
                let mut values = Vec::with_capacity(1968);
                let packed_values = black_box(packed_values);
                for index in 0..1968 {
                    values.push((packed_values[index / 8] >> (index % 8)) & 1 == 1);
                }
                store.write_coils(0, black_box(&values)).await.unwrap();
            });
        },
    );
    group.finish();
}

fn bench_in_memory_store_coil_reads(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = InMemoryStore::new(StoreConfig::default());
    seed_coils(&store, MAX_READ_COIL_COUNT);

    let mut group = c.benchmark_group("in_memory_store_coil_reads");
    group.bench_function("wire_packed_max", |b| {
        b.to_async(&rt).iter(|| async {
            let mut packed = [0u8; MAX_READ_COIL_BYTES];
            store
                .read_coils_packed(0, MAX_READ_COILS, black_box(&mut packed))
                .await
                .unwrap();
            black_box(packed);
        });
    });
    group.bench_function("slice_bool_then_pack_max", |b| {
        b.to_async(&rt).iter(|| async {
            let mut values = [false; MAX_READ_COIL_COUNT];
            let mut packed = [0u8; MAX_READ_COIL_BYTES];
            store
                .read_coils(0, MAX_READ_COILS, black_box(&mut values))
                .await
                .unwrap();
            pack_bool_slice(black_box(&values), &mut packed);
            black_box(packed);
        });
    });
    group.finish();
}

fn bench_in_memory_store_fifo_reads(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = InMemoryStore::new(StoreConfig::default());
    seed_fifo_queue(&store, MAX_FIFO_VALUE_COUNT);

    let mut group = c.benchmark_group("in_memory_store_fifo_reads");
    group.bench_function("wire_be_max", |b| {
        b.to_async(&rt).iter(|| async {
            let mut bytes = [0u8; MAX_FIFO_VALUE_BYTES];
            store
                .read_fifo_queue_be(0, black_box(&mut bytes))
                .await
                .unwrap();
            black_box(bytes);
        });
    });
    group.bench_function("vec_u16_then_pack_max", |b| {
        b.to_async(&rt).iter(|| async {
            let values = store.read_fifo_queue(0).await.unwrap();
            let mut bytes = [0u8; MAX_FIFO_VALUE_BYTES];
            pack_register_slice(black_box(&values), &mut bytes);
            black_box(bytes);
        });
    });
    group.finish();
}

#[allow(clippy::cast_possible_truncation)]
fn register_value_bytes(register_count: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(register_count * 2);
    for register in 0..register_count {
        bytes.extend_from_slice(&(register as u16).to_be_bytes());
    }
    bytes
}

fn seed_coils(store: &InMemoryStore, coil_count: usize) {
    for index in 0..coil_count {
        if index % 3 == 0 {
            let address = u16::try_from(index).expect("benchmark coil address fits u16");
            store.set_coil(address, true).unwrap();
        }
    }
}

fn seed_holding_registers(store: &InMemoryStore, register_count: usize) {
    for index in 0..register_count {
        let address = u16::try_from(index).expect("benchmark register address fits u16");
        let value = u16::try_from(index).expect("benchmark register value fits u16");
        store.set_holding_register(address, value).unwrap();
    }
}

fn seed_fifo_queue(store: &InMemoryStore, value_count: usize) {
    let values = (0..value_count)
        .map(|value| u16::try_from(value).expect("benchmark FIFO value fits u16"))
        .collect();
    store.set_fifo_queue(0, values);
}

fn packed_coil_values(coil_count: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; coil_count.div_ceil(8)];
    for index in 0..coil_count {
        if index % 3 == 0 {
            bytes[index / 8] |= 1 << (index % 8);
        }
    }
    bytes
}

fn pack_bool_slice(bits: &[bool], out: &mut [u8]) {
    out.fill(0);
    for (index, &value) in bits.iter().enumerate() {
        if value {
            out[index / 8] |= 1 << (index % 8);
        }
    }
}

fn pack_register_slice(registers: &[u16], out: &mut [u8]) {
    for (chunk, &value) in out.chunks_exact_mut(2).zip(registers) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
}

criterion_group!(
    benches,
    bench_in_memory_store_register_writes,
    bench_in_memory_store_register_reads,
    bench_in_memory_store_coil_writes,
    bench_in_memory_store_coil_reads,
    bench_in_memory_store_fifo_reads
);
criterion_main!(benches);
