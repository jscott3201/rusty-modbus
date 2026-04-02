"""Tests for SyncModbusClient."""
import pytest
from rusty_modbus import (
    SyncModbusClient,
    ClientConfig,
    ModbusError,
    ConnectionError as RmConnectionError,
)


class TestSyncClientConnection:
    def test_connect_to_nonexistent_server_raises(self):
        """Connecting to a closed port should raise an error."""
        with pytest.raises((RmConnectionError, ModbusError)):
            SyncModbusClient.connect("127.0.0.1:1", config=ClientConfig(timeout_secs=0.5))

    def test_connect_invalid_address_raises_value_error(self):
        with pytest.raises((ValueError, RmConnectionError)):
            SyncModbusClient.connect("not-an-address")

    def test_connect_with_default_config(self):
        """Should accept no config argument (uses defaults)."""
        with pytest.raises((RmConnectionError, ModbusError)):
            SyncModbusClient.connect("127.0.0.1:1")

    def test_connect_tls_invalid_address_raises(self):
        from rusty_modbus import TlsConfig
        tls = TlsConfig(ca_cert="a", client_cert="b", client_key="c")
        with pytest.raises((ValueError, RmConnectionError, ModbusError)):
            SyncModbusClient.connect_tls("not-valid", tls=tls)
