"""Tests for the exception hierarchy."""
import pytest
from rusty_modbus import (
    ModbusError,
    TimeoutError as RmTimeoutError,
    ConnectionError as RmConnectionError,
    ModbusExceptionError,
    RetryError,
)


def test_modbus_error_is_exception():
    assert issubclass(ModbusError, Exception)


def test_timeout_is_modbus_error():
    with pytest.raises(ModbusError):
        raise RmTimeoutError("timed out")


def test_connection_is_modbus_error():
    with pytest.raises(ModbusError):
        raise RmConnectionError("disconnected")


def test_modbus_exception_is_modbus_error():
    with pytest.raises(ModbusError):
        raise ModbusExceptionError("server error")


def test_retry_is_modbus_error():
    with pytest.raises(ModbusError):
        raise RetryError("exhausted")
