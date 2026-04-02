"""Tests for configuration classes."""
import pytest
from rusty_modbus import ClientConfig, RetryConfig, TlsConfig


class TestClientConfig:
    def test_defaults(self):
        cfg = ClientConfig()
        assert cfg.unit_id == 255
        assert cfg.timeout_secs == 5.0
        assert cfg.max_in_flight == 16
        assert cfg.retry is None

    def test_custom_values(self):
        retry = RetryConfig(max_retries=5, retry_delay_ms=200)
        cfg = ClientConfig(unit_id=1, timeout_secs=10.0, max_in_flight=8, retry=retry)
        assert cfg.unit_id == 1
        assert cfg.retry.max_retries == 5

    def test_invalid_timeout(self):
        with pytest.raises(ValueError):
            ClientConfig(timeout_secs=0.0)

    def test_invalid_max_in_flight(self):
        with pytest.raises(ValueError):
            ClientConfig(max_in_flight=0)

    def test_repr(self):
        assert "ClientConfig" in repr(ClientConfig())

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
