//! Frame builders for transport benchmarks.
//!
//! Build complete Frames from codec types for use with raw `TransportSink`/`TransportStream`.

use bytes::Bytes;
use modbus_codec::request::{
    Encode, ReadCoilsRequest, ReadHoldingRegistersRequest, WriteMultipleRegistersRequest,
    WriteSingleRegisterRequest,
};
use modbus_frame::frame::{Frame, FrameHeader};
use modbus_types::{Address, MbapHeader, Quantity};

/// Build an MBAP-framed FC 03 Read Holding Registers request.
pub fn read_holding_registers_mbap(txn_id: u16, unit_id: u8, address: u16, quantity: u16) -> Frame {
    let req = ReadHoldingRegistersRequest {
        address: Address(address),
        quantity: Quantity(quantity),
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(txn_id, unit_id, len as u16)),
        pdu: Bytes::copy_from_slice(&buf[..len]),
    }
}

/// Build an MBAP-framed FC 06 Write Single Register request.
pub fn write_single_register_mbap(txn_id: u16, unit_id: u8, address: u16, value: u16) -> Frame {
    let req = WriteSingleRegisterRequest {
        address: Address(address),
        value,
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(txn_id, unit_id, len as u16)),
        pdu: Bytes::copy_from_slice(&buf[..len]),
    }
}

/// Build an MBAP-framed FC 01 Read Coils request.
pub fn read_coils_mbap(txn_id: u16, unit_id: u8, address: u16, quantity: u16) -> Frame {
    let req = ReadCoilsRequest {
        address: Address(address),
        quantity: Quantity(quantity),
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(txn_id, unit_id, len as u16)),
        pdu: Bytes::copy_from_slice(&buf[..len]),
    }
}

/// Build an MBAP-framed FC 10 Write Multiple Registers request.
pub fn write_multiple_registers_mbap(
    txn_id: u16,
    unit_id: u8,
    address: u16,
    values: &[u16],
) -> Frame {
    let byte_count = (values.len() * 2) as u8;
    let mut value_bytes = vec![0u8; values.len() * 2];
    for (i, &v) in values.iter().enumerate() {
        value_bytes[i * 2..i * 2 + 2].copy_from_slice(&v.to_be_bytes());
    }
    let req = WriteMultipleRegistersRequest {
        address: Address(address),
        quantity: Quantity(values.len() as u16),
        byte_count,
        register_values: &value_bytes,
    };
    let mut buf = [0u8; 256];
    let len = req.encode_into(&mut buf).unwrap();
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(txn_id, unit_id, len as u16)),
        pdu: Bytes::copy_from_slice(&buf[..len]),
    }
}

/// Build an RTU-framed FC 03 Read Holding Registers request.
pub fn read_holding_registers_rtu(unit_id: u8, address: u16, quantity: u16) -> Frame {
    let req = ReadHoldingRegistersRequest {
        address: Address(address),
        quantity: Quantity(quantity),
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    Frame {
        header: FrameHeader::Rtu { unit_id },
        pdu: Bytes::copy_from_slice(&buf[..len]),
    }
}

/// Build an RTU-framed FC 06 Write Single Register request.
pub fn write_single_register_rtu(unit_id: u8, address: u16, value: u16) -> Frame {
    let req = WriteSingleRegisterRequest {
        address: Address(address),
        value,
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    Frame {
        header: FrameHeader::Rtu { unit_id },
        pdu: Bytes::copy_from_slice(&buf[..len]),
    }
}

/// Build an RTU-framed FC 01 Read Coils request.
pub fn read_coils_rtu(unit_id: u8, address: u16, quantity: u16) -> Frame {
    let req = ReadCoilsRequest {
        address: Address(address),
        quantity: Quantity(quantity),
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    Frame {
        header: FrameHeader::Rtu { unit_id },
        pdu: Bytes::copy_from_slice(&buf[..len]),
    }
}

/// Build an RTU-framed FC 10 Write Multiple Registers request.
pub fn write_multiple_registers_rtu(unit_id: u8, address: u16, values: &[u16]) -> Frame {
    let byte_count = (values.len() * 2) as u8;
    let mut value_bytes = vec![0u8; values.len() * 2];
    for (i, &v) in values.iter().enumerate() {
        value_bytes[i * 2..i * 2 + 2].copy_from_slice(&v.to_be_bytes());
    }
    let req = WriteMultipleRegistersRequest {
        address: Address(address),
        quantity: Quantity(values.len() as u16),
        byte_count,
        register_values: &value_bytes,
    };
    let mut buf = [0u8; 256];
    let len = req.encode_into(&mut buf).unwrap();
    Frame {
        header: FrameHeader::Rtu { unit_id },
        pdu: Bytes::copy_from_slice(&buf[..len]),
    }
}
