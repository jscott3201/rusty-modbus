//! Python exception types for Modbus errors.

use pyo3::create_exception;
use pyo3::prelude::*;
use rusty_modbus_client::ClientError;

// Base exception — all rusty_modbus errors subclass this.
create_exception!(rusty_modbus, ModbusError, pyo3::exceptions::PyException);

// All subclass ModbusError so `except ModbusError` catches everything.
create_exception!(rusty_modbus, TimeoutError, ModbusError);
create_exception!(rusty_modbus, ConnectionError, ModbusError);
create_exception!(rusty_modbus, ModbusExceptionError, ModbusError);
create_exception!(rusty_modbus, RetryError, ModbusError);

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
        ClientError::Transport(e) => {
            ConnectionError::new_err(format!("transport error: {e}"))
        }
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
        ClientError::BroadcastReadNotAllowed => {
            ModbusError::new_err("read operations not allowed on broadcast unit ID")
        }
        ClientError::TransactionConflict(id) => {
            ModbusError::new_err(format!("transaction conflict: {:?}", id))
        }
    }
}
