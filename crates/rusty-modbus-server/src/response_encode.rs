//! Server-side response encoding guard.

use rusty_modbus_codec::request::Encode;
use rusty_modbus_codec::response::{
    DiagnosticsResponse, GetCommEventCounterResponse, GetCommEventLogResponse,
    MaskWriteRegisterResponse, ReadCoilsResponse, ReadDiscreteInputsResponse,
    ReadExceptionStatusResponse, ReadFifoQueueResponse, ReadFileRecordResponse,
    ReadHoldingRegistersResponse, ReadInputRegistersResponse, ReadWriteMultipleRegistersResponse,
    ReportServerIdResponse, WriteFileRecordResponse, WriteMultipleCoilsResponse,
    WriteMultipleRegistersResponse, WriteSingleCoilResponse, WriteSingleRegisterResponse,
};
use rusty_modbus_types::{ExceptionCode, FunctionCode};

/// Server response types that have a fixed owning function code.
pub(crate) trait ServerResponse: Encode {
    /// Function code used for normal responses and fallback exceptions.
    const FUNCTION_CODE: FunctionCode;
}

macro_rules! impl_server_response {
    ($($ty:ty => $fc:ident),* $(,)?) => {
        $(
            impl ServerResponse for $ty {
                const FUNCTION_CODE: FunctionCode = FunctionCode::$fc;
            }
        )*
    };
}

impl_server_response! {
    ReadCoilsResponse<'_> => ReadCoils,
    ReadDiscreteInputsResponse<'_> => ReadDiscreteInputs,
    ReadHoldingRegistersResponse<'_> => ReadHoldingRegisters,
    ReadInputRegistersResponse<'_> => ReadInputRegisters,
    WriteSingleCoilResponse => WriteSingleCoil,
    WriteSingleRegisterResponse => WriteSingleRegister,
    ReadExceptionStatusResponse => ReadExceptionStatus,
    DiagnosticsResponse<'_> => Diagnostics,
    GetCommEventCounterResponse => GetCommEventCounter,
    GetCommEventLogResponse<'_> => GetCommEventLog,
    WriteMultipleCoilsResponse => WriteMultipleCoils,
    WriteMultipleRegistersResponse => WriteMultipleRegisters,
    ReportServerIdResponse<'_> => ReportServerId,
    ReadFileRecordResponse<'_> => ReadFileRecord,
    WriteFileRecordResponse<'_> => WriteFileRecord,
    MaskWriteRegisterResponse => MaskWriteRegister,
    ReadWriteMultipleRegistersResponse<'_> => ReadWriteMultipleRegisters,
    ReadFifoQueueResponse<'_> => ReadFifoQueue,
}

/// Encode a normal response, falling back to `ServerDeviceFailure` on invariant bugs.
pub(crate) fn encode_response<R: ServerResponse>(resp: &R) -> Vec<u8> {
    let mut buf = vec![0u8; resp.encoded_len()];
    match resp.encode_into(&mut buf) {
        Ok(written) if written == buf.len() => buf,
        Ok(_) | Err(_) => encode_exception(R::FUNCTION_CODE.exception_code()),
    }
}

fn encode_exception(fc_with_flag: u8) -> Vec<u8> {
    vec![fc_with_flag, ExceptionCode::ServerDeviceFailure.code()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_modbus_codec::EncodeError;

    struct FailingResponse;

    impl Encode for FailingResponse {
        fn encode_into(&self, _buf: &mut [u8]) -> Result<usize, EncodeError> {
            Err(EncodeError::ByteCountMismatch {
                declared: 1,
                actual: 0,
            })
        }

        fn encoded_len(&self) -> usize {
            2
        }
    }

    impl ServerResponse for FailingResponse {
        const FUNCTION_CODE: FunctionCode = FunctionCode::ReadHoldingRegisters;
    }

    struct ShortWriteResponse;

    impl Encode for ShortWriteResponse {
        fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
            buf[0] = FunctionCode::ReadHoldingRegisters.code();
            Ok(1)
        }

        fn encoded_len(&self) -> usize {
            2
        }
    }

    impl ServerResponse for ShortWriteResponse {
        const FUNCTION_CODE: FunctionCode = FunctionCode::ReadHoldingRegisters;
    }

    #[test]
    fn encode_response_preserves_successful_response() {
        let resp = ReadExceptionStatusResponse { status: 0x5A };

        assert_eq!(encode_response(&resp), vec![0x07, 0x5A]);
    }

    #[test]
    fn encode_response_maps_encode_error_to_server_device_failure() {
        assert_eq!(
            encode_response(&FailingResponse),
            vec![0x83, ExceptionCode::ServerDeviceFailure.code()]
        );
    }

    #[test]
    fn encode_response_maps_short_write_to_server_device_failure() {
        assert_eq!(
            encode_response(&ShortWriteResponse),
            vec![0x83, ExceptionCode::ServerDeviceFailure.code()]
        );
    }
}
