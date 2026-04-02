"""Tests for async ModbusClient."""
import pytest
from rusty_modbus import (
    ModbusClient,
    ClientConfig,
    ModbusError,
    ConnectionError as RmConnectionError,
)


@pytest.mark.asyncio
async def test_connect_to_nonexistent_server_raises():
    with pytest.raises((RmConnectionError, ModbusError)):
        await ModbusClient.connect(
            "127.0.0.1:1",
            config=ClientConfig(timeout_secs=0.5),
        )


@pytest.mark.asyncio
async def test_connect_invalid_address_raises():
    with pytest.raises((ValueError, RmConnectionError)):
        await ModbusClient.connect("not-an-address")


@pytest.mark.asyncio
async def test_connect_tls_invalid_address_raises():
    from rusty_modbus import TlsConfig
    tls = TlsConfig(ca_cert="a", client_cert="b", client_key="c")
    with pytest.raises((ValueError, RmConnectionError, ModbusError)):
        await ModbusClient.connect_tls("not-valid", tls=tls)
