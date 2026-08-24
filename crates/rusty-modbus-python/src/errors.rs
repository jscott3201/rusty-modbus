//! Python exception types for Modbus errors.

use pyo3::create_exception;
use pyo3::prelude::*;
use rusty_modbus_client::ClientError;
use rusty_modbus_server::ServerError;
use rusty_modbus_types::ExceptionCode;

// Base exception — all rusty_modbus errors subclass this.
create_exception!(rusty_modbus, ModbusError, pyo3::exceptions::PyException);

// All subclass ModbusError so `except ModbusError` catches everything.
create_exception!(rusty_modbus, TimeoutError, ModbusError);
create_exception!(rusty_modbus, ConnectionError, ModbusError);
create_exception!(rusty_modbus, ModbusExceptionError, ModbusError);
create_exception!(rusty_modbus, RetryError, ModbusError);
create_exception!(rusty_modbus, IllegalFunctionError, ModbusError);
create_exception!(rusty_modbus, IllegalDataAddressError, ModbusError);
create_exception!(rusty_modbus, IllegalDataValueError, ModbusError);
create_exception!(rusty_modbus, ServerDeviceFailureError, ModbusError);
create_exception!(rusty_modbus, AcknowledgeError, ModbusError);
create_exception!(rusty_modbus, ServerDeviceBusyError, ModbusError);
create_exception!(rusty_modbus, NegativeAcknowledgeError, ModbusError);
create_exception!(rusty_modbus, MemoryParityError, ModbusError);
create_exception!(rusty_modbus, GatewayPathUnavailableError, ModbusError);
create_exception!(
    rusty_modbus,
    GatewayTargetDeviceFailedToRespondError,
    ModbusError
);

/// Register all exception types on the module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("ModbusError", m.py().get_type::<ModbusError>())?;
    m.add("TimeoutError", m.py().get_type::<TimeoutError>())?;
    m.add("ConnectionError", m.py().get_type::<ConnectionError>())?;
    m.add(
        "ModbusExceptionError",
        m.py().get_type::<ModbusExceptionError>(),
    )?;
    m.add("RetryError", m.py().get_type::<RetryError>())?;
    m.add(
        "IllegalFunctionError",
        m.py().get_type::<IllegalFunctionError>(),
    )?;
    m.add(
        "IllegalDataAddressError",
        m.py().get_type::<IllegalDataAddressError>(),
    )?;
    m.add(
        "IllegalDataValueError",
        m.py().get_type::<IllegalDataValueError>(),
    )?;
    m.add(
        "ServerDeviceFailureError",
        m.py().get_type::<ServerDeviceFailureError>(),
    )?;
    m.add("AcknowledgeError", m.py().get_type::<AcknowledgeError>())?;
    m.add(
        "ServerDeviceBusyError",
        m.py().get_type::<ServerDeviceBusyError>(),
    )?;
    m.add(
        "NegativeAcknowledgeError",
        m.py().get_type::<NegativeAcknowledgeError>(),
    )?;
    m.add("MemoryParityError", m.py().get_type::<MemoryParityError>())?;
    m.add(
        "GatewayPathUnavailableError",
        m.py().get_type::<GatewayPathUnavailableError>(),
    )?;
    m.add(
        "GatewayTargetDeviceFailedToRespondError",
        m.py().get_type::<GatewayTargetDeviceFailedToRespondError>(),
    )?;
    Ok(())
}

/// Convert a `ClientError` into the appropriate Python exception.
pub fn client_error_to_pyerr(err: ClientError) -> PyErr {
    match err {
        ClientError::Timeout => TimeoutError::new_err("request timed out"),
        ClientError::Exception(exc) => {
            let code = exc.exception_code.code();
            let msg = format!(
                "Modbus exception 0x{:02X}: {:?} (FC 0x{:02X})",
                code,
                exc.exception_code,
                exc.function_code.code(),
            );
            ModbusExceptionError::new_err((msg, code))
        }
        ClientError::Transport(e) => ConnectionError::new_err(format!("transport error: {e}")),
        ClientError::NotConnected => ConnectionError::new_err("not connected"),
        ClientError::ShuttingDown => ConnectionError::new_err("client is shutting down"),
        ClientError::RetriesExhausted {
            attempts,
            last_error,
        } => {
            let msg = format!("retries exhausted after {attempts} attempts: {last_error}");
            RetryError::new_err((msg, attempts))
        }
        ClientError::Codec(e) => ModbusError::new_err(format!("codec error: {e}")),
        ClientError::Encode(e) => ModbusError::new_err(format!("request encode error: {e}")),
        ClientError::BroadcastReadNotAllowed => {
            ModbusError::new_err("read operations not allowed on broadcast unit ID")
        }
        ClientError::TransactionConflict(id) => {
            ModbusError::new_err(format!("transaction conflict: {:?}", id))
        }
        ClientError::UnexpectedResponseUnitId { expected, got } => ModbusError::new_err(format!(
            "unexpected response unit ID: expected 0x{expected:02X}, got 0x{got:02X}"
        )),
        ClientError::UnexpectedResponse { expected, got } => ModbusError::new_err(format!(
            "unexpected response function code: expected 0x{expected:02X}, got 0x{got:02X}"
        )),
        ClientError::UnexpectedResponseEcho {
            field,
            expected,
            got,
        } => ModbusError::new_err(format!(
            "unexpected response echo for {field}: expected 0x{expected:04X}, got 0x{got:04X}"
        )),
        ClientError::ShortResponse { expected, actual } => ModbusError::new_err(format!(
            "short response: need {expected} data bytes for the request, got {actual}"
        )),
        ClientError::UnexpectedResponseLength {
            function_code,
            expected,
            actual,
        } => ModbusError::new_err(format!(
            "unexpected response length for function 0x{function_code:02X}: expected {expected} data bytes, got {actual}"
        )),
        ClientError::UnexpectedResponsePadding {
            function_code,
            invalid_mask,
            actual,
        } => ModbusError::new_err(format!(
            "unexpected response padding for function 0x{function_code:02X}: byte 0x{actual:02X} sets bits selected by 0x{invalid_mask:02X}"
        )),
        ClientError::InvalidDeviceIdentificationContinuation {
            previous_object_id,
            next_object_id,
        } => ModbusError::new_err(format!(
            "invalid device identification continuation: next object ID 0x{next_object_id:02X} did not advance past 0x{previous_object_id:02X}"
        )),
        ClientError::DeviceIdentificationPaginationLimit { limit } => ModbusError::new_err(
            format!("device identification pagination exceeded {limit} response pages"),
        ),
    }
}

/// Convert a `ServerError` into the appropriate Python exception.
pub fn server_error_to_pyerr(err: ServerError) -> PyErr {
    match err {
        ServerError::Bind(e) => ConnectionError::new_err(format!("bind failed: {e}")),
        ServerError::Transport(e) => ConnectionError::new_err(format!("transport error: {e}")),
        ServerError::AlreadyRunning => ModbusError::new_err("server is already running"),
    }
}

/// Convert a Python data-store exception into the Modbus exception code sent on
/// the wire. Unknown Python exceptions become ServerDeviceFailure.
pub fn pyerr_to_exception_code(py: Python<'_>, err: PyErr) -> ExceptionCode {
    if err.is_instance_of::<IllegalFunctionError>(py) {
        ExceptionCode::IllegalFunction
    } else if err.is_instance_of::<IllegalDataAddressError>(py) {
        ExceptionCode::IllegalDataAddress
    } else if err.is_instance_of::<IllegalDataValueError>(py) {
        ExceptionCode::IllegalDataValue
    } else if err.is_instance_of::<ServerDeviceFailureError>(py) {
        ExceptionCode::ServerDeviceFailure
    } else if err.is_instance_of::<AcknowledgeError>(py) {
        ExceptionCode::Acknowledge
    } else if err.is_instance_of::<ServerDeviceBusyError>(py) {
        ExceptionCode::ServerDeviceBusy
    } else if err.is_instance_of::<NegativeAcknowledgeError>(py) {
        ExceptionCode::NegativeAcknowledge
    } else if err.is_instance_of::<MemoryParityError>(py) {
        ExceptionCode::MemoryParityError
    } else if err.is_instance_of::<GatewayPathUnavailableError>(py) {
        ExceptionCode::GatewayPathUnavailable
    } else if err.is_instance_of::<GatewayTargetDeviceFailedToRespondError>(py) {
        ExceptionCode::GatewayTargetDeviceFailedToRespond
    } else {
        ExceptionCode::ServerDeviceFailure
    }
}
