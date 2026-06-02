//! Codec encode/decode, frame decode, and CRC-16 micro-benchmarks.

use bytes::{Bytes, BytesMut};
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use rusty_modbus_codec::request::{
    Encode, ReadHoldingRegistersRequest, WriteMultipleCoilsRequest, WriteMultipleRegistersRequest,
};
use rusty_modbus_codec::response::{ReadCoilsResponse, ReadHoldingRegistersResponse};
use rusty_modbus_codec::{RequestPdu, ResponsePdu, decode_request, decode_response};
use rusty_modbus_frame::crc::crc16;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_frame::mbap::MbapCodec;
use rusty_modbus_frame::owned::OwnedResponsePdu;
use rusty_modbus_types::{Address, MbapHeader, Quantity};
use tokio_util::codec::{Decoder, Encoder};

// ── Encode benchmarks ────────────────────────────────────────────

fn bench_encode_read_holding_registers(c: &mut Criterion) {
    let req = ReadHoldingRegistersRequest {
        address: Address(0),
        quantity: Quantity(10),
    };
    c.bench_function("encode_read_holding_registers", |b| {
        b.iter(|| {
            let mut buf = [0u8; 5];
            black_box(req.encode_into(&mut buf).unwrap());
        });
    });
}

fn bench_encode_write_multiple_registers(c: &mut Criterion) {
    let values = [0u8; 20]; // 10 registers
    let req = WriteMultipleRegistersRequest {
        address: Address(0),
        quantity: Quantity(10),
        byte_count: 20,
        register_values: &values,
    };
    c.bench_function("encode_write_multiple_registers", |b| {
        b.iter(|| {
            let mut buf = [0u8; 256];
            black_box(req.encode_into(&mut buf).unwrap());
        });
    });
}

fn bench_encode_write_multiple_coils(c: &mut Criterion) {
    let coil_bytes = [0xFF; 125]; // 1000 coils
    let req = WriteMultipleCoilsRequest {
        address: Address(0),
        quantity: Quantity(1000),
        byte_count: 125,
        coil_values: &coil_bytes,
    };
    c.bench_function("encode_write_multiple_coils", |b| {
        b.iter(|| {
            let mut buf = [0u8; 256];
            black_box(req.encode_into(&mut buf).unwrap());
        });
    });
}

fn bench_encode_full_pdu_with_mbap(c: &mut Criterion) {
    let req = ReadHoldingRegistersRequest {
        address: Address(0),
        quantity: Quantity(10),
    };
    c.bench_function("encode_full_pdu_with_mbap", |b| {
        b.iter(|| {
            let mut pdu_buf = [0u8; 5];
            let pdu_len = req.encode_into(&mut pdu_buf).unwrap();
            let header = MbapHeader::new(1, 1, pdu_len as u16);
            let frame = Frame {
                header: FrameHeader::Mbap(header),
                pdu: bytes::Bytes::copy_from_slice(&pdu_buf[..pdu_len]),
            };
            let mut dst = BytesMut::with_capacity(12);
            let mut codec = MbapCodec;
            codec.encode(frame, &mut dst).unwrap();
            black_box(dst);
        });
    });
}

// ── Decode benchmarks ────────────────────────────────────────────

fn bench_decode_read_holding_registers_response(c: &mut Criterion) {
    let mut data = vec![20u8]; // byte_count
    data.extend_from_slice(&[0u8; 20]); // 10 registers

    c.bench_function("decode_read_holding_registers_response", |b| {
        b.iter(|| {
            black_box(ReadHoldingRegistersResponse::decode(black_box(&data)).unwrap());
        });
    });
}

fn bench_decode_read_coils_response(c: &mut Criterion) {
    let mut data = vec![13u8]; // byte_count = ceil(100/8)
    data.extend_from_slice(&[0xAA; 13]);

    c.bench_function("decode_read_coils_response", |b| {
        b.iter(|| {
            black_box(ReadCoilsResponse::decode(black_box(&data)).unwrap());
        });
    });
}

fn bench_decode_read_holding_registers_request(c: &mut Criterion) {
    let data = [0x00, 0x6B, 0x00, 0x7D];

    c.bench_function("decode_read_holding_registers_request", |b| {
        b.iter(|| {
            black_box(ReadHoldingRegistersRequest::decode(black_box(&data)).unwrap());
        });
    });
}

fn bench_decode_write_multiple_registers_request_max(c: &mut Criterion) {
    let data = write_multiple_registers_request_data(123);

    c.bench_function("decode_write_multiple_registers_request_max", |b| {
        b.iter(|| {
            black_box(WriteMultipleRegistersRequest::decode(black_box(&data)).unwrap());
        });
    });
}

fn bench_decode_request_dispatch(c: &mut Criterion) {
    let read_holding = [0x03, 0x00, 0x6B, 0x00, 0x7D];
    let write_multiple = write_multiple_registers_request_pdu(123);

    let mut group = c.benchmark_group("decode_request_dispatch");
    group.bench_with_input(
        BenchmarkId::from_parameter("read_holding_registers"),
        &read_holding[..],
        |b, pdu| {
            b.iter(|| {
                let decoded = decode_request(black_box(pdu)).unwrap();
                match decoded {
                    RequestPdu::ReadHoldingRegisters(req) => black_box(req.quantity.0),
                    _ => unreachable!("unexpected request variant"),
                }
            });
        },
    );
    group.bench_with_input(
        BenchmarkId::from_parameter("write_multiple_registers_max"),
        &write_multiple[..],
        |b, pdu| {
            b.iter(|| {
                let decoded = decode_request(black_box(pdu)).unwrap();
                match decoded {
                    RequestPdu::WriteMultipleRegisters(req) => {
                        black_box((req.quantity.0, req.register_values.len()));
                    }
                    _ => unreachable!("unexpected request variant"),
                }
            });
        },
    );
    group.finish();
}

fn bench_decode_read_holding_registers_response_max(c: &mut Criterion) {
    let data = read_holding_registers_response_data(125);

    c.bench_function("decode_read_holding_registers_response_max", |b| {
        b.iter(|| {
            black_box(ReadHoldingRegistersResponse::decode(black_box(&data)).unwrap());
        });
    });
}

fn bench_decode_read_holding_registers_response_max_iterate(c: &mut Criterion) {
    let data = read_holding_registers_response_data(125);

    c.bench_function("decode_read_holding_registers_response_max_iterate", |b| {
        b.iter(|| {
            let response = ReadHoldingRegistersResponse::decode(black_box(&data)).unwrap();
            let sum = response
                .registers()
                .fold(0u32, |acc, value| acc + u32::from(value));
            black_box(sum);
        });
    });
}

fn bench_decode_response_dispatch(c: &mut Criterion) {
    let pdu = read_holding_registers_response_pdu(125);

    c.bench_function("decode_response_dispatch_read_holding_registers_max", |b| {
        b.iter(|| {
            let decoded = decode_response(black_box(&pdu)).unwrap();
            match decoded {
                ResponsePdu::ReadHoldingRegisters(response) => {
                    black_box((response.byte_count, response.count()));
                }
                _ => unreachable!("unexpected response variant"),
            }
        });
    });
}

fn bench_decode_owned_response_dispatch(c: &mut Criterion) {
    let pdu = Bytes::from(read_holding_registers_response_pdu(125));

    c.bench_function(
        "decode_owned_response_dispatch_read_holding_registers_max",
        |b| {
            b.iter(|| {
                let decoded = OwnedResponsePdu::from_pdu(black_box(pdu.clone())).unwrap();
                match decoded {
                    OwnedResponsePdu::ReadHoldingRegisters(response) => {
                        black_box((response.byte_count, response.count()));
                    }
                    _ => unreachable!("unexpected response variant"),
                }
            });
        },
    );
}

fn bench_unpack_write_payloads(c: &mut Criterion) {
    let register_values = register_value_bytes(123);
    let coil_values = [0xAA; 125];

    let mut group = c.benchmark_group("unpack_write_payloads");
    group.bench_function("registers_max_to_vec_u16", |b| {
        b.iter(|| {
            let values: Vec<u16> = black_box(&register_values)
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            black_box(values);
        });
    });
    group.bench_function("coils_max_to_vec_bool", |b| {
        b.iter(|| {
            let mut values = Vec::with_capacity(1000);
            let coil_values = black_box(&coil_values);
            for i in 0..1000 {
                values.push((coil_values[i / 8] >> (i % 8)) & 1 == 1);
            }
            black_box(values);
        });
    });
    group.finish();
}

// ── CRC-16 benchmarks ───────────────────────────────────────────

fn bench_crc16(c: &mut Criterion) {
    let mut group = c.benchmark_group("crc16");

    let small = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD];
    let medium = [0xAAu8; 64];
    let large = vec![0x55u8; 253];

    group.bench_with_input(BenchmarkId::new("small", "8B"), &small[..], |b, data| {
        b.iter(|| black_box(crc16(black_box(data))));
    });
    group.bench_with_input(BenchmarkId::new("medium", "64B"), &medium[..], |b, data| {
        b.iter(|| black_box(crc16(black_box(data))));
    });
    group.bench_with_input(BenchmarkId::new("large", "253B"), &large[..], |b, data| {
        b.iter(|| black_box(crc16(black_box(data))));
    });

    group.finish();
}

fn bench_verify_crc_frame(c: &mut Criterion) {
    let data = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A];
    let crc = crc16(&data);
    let frame = [&data[..], &crc.to_le_bytes()].concat();

    c.bench_function("verify_crc_frame", |b| {
        b.iter(|| black_box(rusty_modbus_frame::crc::verify_crc(black_box(&frame))));
    });
}

// ── MBAP framing benchmarks ─────────────────────────────────────

fn bench_mbap_encode_frame(c: &mut Criterion) {
    let pdu = bytes::Bytes::from_static(&[0x03, 0x00, 0x6B, 0x00, 0x03]);
    let header = MbapHeader::new(1, 0xFF, pdu.len() as u16);

    c.bench_function("mbap_encode_frame", |b| {
        b.iter(|| {
            let frame = Frame {
                header: FrameHeader::Mbap(header),
                pdu: pdu.clone(),
            };
            let mut dst = BytesMut::with_capacity(12);
            let mut codec = MbapCodec;
            codec.encode(frame, &mut dst).unwrap();
            black_box(dst);
        });
    });
}

fn bench_mbap_decode_frame(c: &mut Criterion) {
    let pdu = bytes::Bytes::from_static(&[0x03, 0x00, 0x6B, 0x00, 0x03]);
    let header = MbapHeader::new(1, 0xFF, pdu.len() as u16);
    let frame = Frame {
        header: FrameHeader::Mbap(header),
        pdu,
    };
    let mut encoded = BytesMut::new();
    let mut codec = MbapCodec;
    codec.encode(frame, &mut encoded).unwrap();
    let wire_bytes = encoded.freeze();

    c.bench_function("mbap_decode_frame", |b| {
        b.iter(|| {
            let mut buf = BytesMut::from(wire_bytes.as_ref());
            let mut codec = MbapCodec;
            black_box(codec.decode(&mut buf).unwrap().unwrap());
        });
    });
}

fn bench_mbap_decode_frame_reused_buffer(c: &mut Criterion) {
    let pdu = bytes::Bytes::from_static(&[0x03, 0x00, 0x6B, 0x00, 0x03]);
    let header = MbapHeader::new(1, 0xFF, pdu.len() as u16);
    let frame = Frame {
        header: FrameHeader::Mbap(header),
        pdu,
    };
    let mut encoded = BytesMut::new();
    let mut codec = MbapCodec;
    codec.encode(frame, &mut encoded).unwrap();
    let wire_bytes = encoded.freeze();

    c.bench_function("mbap_decode_frame_reused_buffer", |b| {
        let mut buf = BytesMut::with_capacity(wire_bytes.len());
        let mut codec = MbapCodec;
        b.iter(|| {
            buf.clear();
            buf.extend_from_slice(wire_bytes.as_ref());
            black_box(codec.decode(&mut buf).unwrap().unwrap());
        });
    });
}

#[allow(clippy::cast_possible_truncation)]
fn read_holding_registers_response_data(register_count: usize) -> Vec<u8> {
    let byte_count = register_count * 2;
    let mut data = Vec::with_capacity(1 + byte_count);
    data.push(byte_count as u8);
    for register in 0..register_count {
        data.extend_from_slice(&(register as u16).to_be_bytes());
    }
    data
}

fn read_holding_registers_response_pdu(register_count: usize) -> Vec<u8> {
    let mut pdu = Vec::with_capacity(1 + 1 + register_count * 2);
    pdu.push(0x03);
    pdu.extend_from_slice(&read_holding_registers_response_data(register_count));
    pdu
}

#[allow(clippy::cast_possible_truncation)]
fn write_multiple_registers_request_data(register_count: usize) -> Vec<u8> {
    let byte_count = register_count * 2;
    let mut data = Vec::with_capacity(5 + byte_count);
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&(register_count as u16).to_be_bytes());
    data.push(byte_count as u8);
    data.extend_from_slice(&register_value_bytes(register_count));
    data
}

fn write_multiple_registers_request_pdu(register_count: usize) -> Vec<u8> {
    let mut pdu = Vec::with_capacity(1 + 5 + register_count * 2);
    pdu.push(0x10);
    pdu.extend_from_slice(&write_multiple_registers_request_data(register_count));
    pdu
}

#[allow(clippy::cast_possible_truncation)]
fn register_value_bytes(register_count: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(register_count * 2);
    for register in 0..register_count {
        bytes.extend_from_slice(&(register as u16).to_be_bytes());
    }
    bytes
}

criterion_group!(
    encode,
    bench_encode_read_holding_registers,
    bench_encode_write_multiple_registers,
    bench_encode_write_multiple_coils,
    bench_encode_full_pdu_with_mbap,
);
criterion_group!(
    decode,
    bench_decode_read_holding_registers_response,
    bench_decode_read_coils_response,
    bench_decode_read_holding_registers_request,
    bench_decode_write_multiple_registers_request_max,
    bench_decode_request_dispatch,
    bench_decode_read_holding_registers_response_max,
    bench_decode_read_holding_registers_response_max_iterate,
    bench_decode_response_dispatch,
    bench_decode_owned_response_dispatch,
    bench_unpack_write_payloads,
);
criterion_group!(crc, bench_crc16, bench_verify_crc_frame);
criterion_group!(
    mbap,
    bench_mbap_encode_frame,
    bench_mbap_decode_frame,
    bench_mbap_decode_frame_reused_buffer
);
criterion_main!(encode, decode, crc, mbap);
