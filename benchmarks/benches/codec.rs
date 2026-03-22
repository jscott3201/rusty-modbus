//! Codec encode/decode and CRC-16 micro-benchmarks.

use bytes::BytesMut;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use modbus_codec::request::{
    Encode, ReadCoilsRequest, ReadHoldingRegistersRequest, WriteMultipleCoilsRequest,
    WriteMultipleRegistersRequest,
};
use modbus_codec::response::{ReadCoilsResponse, ReadHoldingRegistersResponse};
use modbus_frame::crc::crc16;
use modbus_frame::frame::{Frame, FrameHeader};
use modbus_frame::mbap::MbapCodec;
use modbus_types::{Address, MbapHeader, Quantity};
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

// ── CRC-16 benchmarks ───────────────────────────────────────────

fn bench_crc16(c: &mut Criterion) {
    let mut group = c.benchmark_group("crc16");

    let small = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD];
    let medium = vec![0xAAu8; 64];
    let large = vec![0x55u8; 253];

    group.bench_with_input(BenchmarkId::new("small", "8B"), &small[..], |b, data| {
        b.iter(|| black_box(crc16(black_box(data))));
    });
    group.bench_with_input(
        BenchmarkId::new("medium", "64B"),
        &medium[..],
        |b, data| {
            b.iter(|| black_box(crc16(black_box(data))));
        },
    );
    group.bench_with_input(
        BenchmarkId::new("large", "253B"),
        &large[..],
        |b, data| {
            b.iter(|| black_box(crc16(black_box(data))));
        },
    );

    group.finish();
}

fn bench_verify_crc_frame(c: &mut Criterion) {
    let data = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A];
    let crc = crc16(&data);
    let frame = [&data[..], &crc.to_le_bytes()].concat();

    c.bench_function("verify_crc_frame", |b| {
        b.iter(|| black_box(modbus_frame::crc::verify_crc(black_box(&frame))));
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
);
criterion_group!(crc, bench_crc16, bench_verify_crc_frame);
criterion_group!(mbap, bench_mbap_encode_frame, bench_mbap_decode_frame);
criterion_main!(encode, decode, crc, mbap);
