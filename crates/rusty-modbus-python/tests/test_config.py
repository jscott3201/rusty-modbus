"""Tests for configuration classes."""
import pytest
from rusty_modbus import ClientConfig, RetryConfig, ServerConfig, StoreConfig, TlsConfig


class TestClientConfig:
    def test_defaults(self):
        cfg = ClientConfig()
        assert cfg.unit_id == 255
        assert cfg.timeout_secs == 5.0
        assert cfg.max_in_flight == 16
        assert cfg.retry is None
        assert cfg.shutdown_timeout_secs == 10.0

    def test_custom_values(self):
        retry = RetryConfig(max_retries=5, retry_delay_ms=200)
        cfg = ClientConfig(
            unit_id=1,
            timeout_secs=10.0,
            max_in_flight=8,
            retry=retry,
            shutdown_timeout_secs=2.5,
        )
        assert cfg.unit_id == 1
        assert cfg.retry.max_retries == 5
        assert cfg.shutdown_timeout_secs == 2.5

    @pytest.mark.parametrize("value", [0.0, -1.0, float("inf"), float("nan")])
    def test_invalid_timeout(self, value):
        with pytest.raises(ValueError, match="timeout_secs"):
            ClientConfig(timeout_secs=value)

    def test_invalid_max_in_flight(self):
        with pytest.raises(ValueError):
            ClientConfig(max_in_flight=0)

    @pytest.mark.parametrize("value", [0.0, -1.0, float("inf"), float("nan")])
    def test_invalid_shutdown_timeout(self, value):
        with pytest.raises(ValueError, match="shutdown_timeout_secs"):
            ClientConfig(shutdown_timeout_secs=value)

    def test_repr(self):
        text = repr(ClientConfig())
        assert "ClientConfig" in text
        assert "shutdown_timeout_secs=10" in text

    def test_frozen(self):
        cfg = ClientConfig()
        with pytest.raises(AttributeError):
            cfg.unit_id = 1


class TestTlsConfig:
    def test_construction(self):
        cfg = TlsConfig(ca_cert="/tmp/ca.pem", client_cert="/tmp/client.pem", client_key="/tmp/key.pem")
        assert cfg.ca_cert == "/tmp/ca.pem"

    def test_repr_hides_key(self):
        cfg = TlsConfig(ca_cert="a", client_cert="b", client_key="secret")
        assert "secret" not in repr(cfg)


class TestStoreConfig:
    def test_rejects_oversized_table(self):
        with pytest.raises(ValueError, match="holding_registers"):
            StoreConfig(holding_register_count=65_537)


class TestServerConfig:
    @pytest.mark.parametrize("value", [0.0, -1.0, float("inf"), float("nan"), 1e300])
    def test_invalid_shutdown_timeout(self, value):
        with pytest.raises(ValueError, match="shutdown_timeout_secs"):
            ServerConfig(shutdown_timeout_secs=value)

    def test_transaction_limit_above_client_ring_size_is_accepted(self):
        assert ServerConfig(max_transactions=17).max_transactions == 17
