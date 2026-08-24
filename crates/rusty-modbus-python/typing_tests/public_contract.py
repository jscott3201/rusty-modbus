from __future__ import annotations

from collections.abc import Awaitable, Sequence
from typing import TYPE_CHECKING, Literal, assert_type

import rusty_modbus

if TYPE_CHECKING:
    from rusty_modbus import (
        DataStore,
        FifoDataStore,
        FileRecordDataStore,
        SerialDiagnosticsDataStore,
        ServerIdentificationDataStore,
    )


class CompleteStore:
    def read_coils(self, address: int, quantity: int) -> Sequence[bool]:
        return [False] * quantity

    def write_coil(self, address: int, value: bool) -> None:
        return None

    def write_coils(self, address: int, values: Sequence[bool]) -> None:
        return None

    def read_discrete_inputs(self, address: int, quantity: int) -> Sequence[bool]:
        return [False] * quantity

    def read_holding_registers(self, address: int, quantity: int) -> Sequence[int]:
        return [0] * quantity

    def write_register(self, address: int, value: int) -> None:
        return None

    def write_registers(self, address: int, values: Sequence[int]) -> None:
        return None

    def read_input_registers(self, address: int, quantity: int) -> Sequence[int]:
        return [0] * quantity

    def read_file_record(
        self, file_number: int, record_number: int, record_length: int
    ) -> Sequence[int]:
        return [0] * record_length

    def write_file_record(
        self, file_number: int, record_number: int, values: Sequence[int]
    ) -> None:
        return None

    def read_fifo_queue(self, address: int) -> Sequence[int]:
        return [0]

    def read_exception_status(self) -> int:
        return 0

    def get_comm_event_counter(self) -> tuple[int, int]:
        return (0, 0)

    def get_comm_event_log(self) -> tuple[int, int, int, bytes]:
        return (0, 0, 0, b"")

    def diagnostic(self, sub_function: int, data: Sequence[int]) -> bytes | None:
        return bytes(data)

    def report_server_id(self) -> bytes:
        return b"rusty-modbus"


def _config_properties_are_read_only() -> None:
    client_config = rusty_modbus.ClientConfig()
    assert_type(client_config.unit_id, int)
    assert_type(client_config.timeout_secs, float)
    assert_type(client_config.shutdown_timeout_secs, float)
    assert_type(client_config.retry, rusty_modbus.RetryConfig | None)
    client_config.unit_id = 1  # pyright: ignore[reportAttributeAccessIssue]

    retry_config = rusty_modbus.RetryConfig()
    assert_type(retry_config.max_retries, int)
    retry_config.max_retries = 5  # pyright: ignore[reportAttributeAccessIssue]

    server_config = rusty_modbus.ServerConfig()
    assert_type(server_config.listen_addr, str)
    server_config.listen_addr = "127.0.0.1:502"  # pyright: ignore[reportAttributeAccessIssue]

    store_config = rusty_modbus.StoreConfig()
    assert_type(store_config.holding_register_count, int)
    store_config.holding_register_count = 1  # pyright: ignore[reportAttributeAccessIssue]


def _client_contracts(
    async_client: rusty_modbus.ModbusClient,
    sync_client: rusty_modbus.SyncModbusClient,
) -> None:
    assert_type(
        rusty_modbus.ModbusClient.connect("127.0.0.1:502"),
        Awaitable[rusty_modbus.ModbusClient],
    )
    assert_type(async_client.read_holding_registers(1, 0, 2), Awaitable[list[int]])
    assert_type(async_client.write_file_record(1, bytearray([6, 0, 1])), Awaitable[bytes])
    assert_type(async_client.abort(), None)

    assert_type(
        rusty_modbus.SyncModbusClient.connect("127.0.0.1:502"),
        rusty_modbus.SyncModbusClient,
    )
    assert_type(sync_client.read_coils(1, 0, 2), list[bool])
    assert_type(sync_client.write_file_record(1, (6, 0, 1)), bytes)
    assert_type(sync_client.abort(), None)
    assert_type(sync_client.__exit__(None, None, None), bool)


def _server_contracts(store: DataStore) -> None:
    assert_type(
        rusty_modbus.ModbusServer.start(store=rusty_modbus.InMemoryStore()),
        rusty_modbus.ModbusServer,
    )
    assert_type(rusty_modbus.ModbusServer.start(store=store), rusty_modbus.ModbusServer)


def _server_lifecycle_contracts(server: rusty_modbus.ModbusServer) -> None:
    assert_type(server.stop(), Literal["drained", "forced"])
    metrics = server.metrics()
    assert_type(metrics, rusty_modbus.ServerMetrics)
    assert_type(metrics.active_connections, int)
    metrics.active_connections = 1  # pyright: ignore[reportAttributeAccessIssue]


def _protocol_contracts(
    required: DataStore,
    file_records: FileRecordDataStore,
    fifo: FifoDataStore,
    serial: SerialDiagnosticsDataStore,
    server_id: ServerIdentificationDataStore,
) -> None:
    assert_type(required, DataStore)
    assert_type(file_records, FileRecordDataStore)
    assert_type(fifo, FifoDataStore)
    assert_type(serial, SerialDiagnosticsDataStore)
    assert_type(server_id, ServerIdentificationDataStore)

    complete = CompleteStore()
    assert_type(complete, CompleteStore)
    _server_contracts(complete)
