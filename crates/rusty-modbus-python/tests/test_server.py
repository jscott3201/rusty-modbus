"""Tests for Modbus server bindings."""

import pytest

from rusty_modbus import (
    IllegalDataAddressError,
    InMemoryStore,
    ModbusExceptionError,
    ModbusServer,
    ServerConfig,
    StoreConfig,
    SyncModbusClient,
)


def test_in_memory_server_round_trip():
    store = InMemoryStore(StoreConfig())
    store.set_holding_register(0, 123)
    store.set_coil(0, True)
    store.set_fifo_queue(0, [10, 11])

    with ModbusServer.start(ServerConfig(), store) as server:
        with SyncModbusClient.connect(server.local_addr) as client:
            assert client.read_holding_registers(unit_id=1, address=0, quantity=1) == [123]
            assert client.read_coils(unit_id=1, address=0, quantity=1) == [True]
            assert client.read_fifo_queue(unit_id=1, pointer_address=0) == [10, 11]

            client.write_single_register(unit_id=1, address=1, value=456)
            assert client.read_holding_registers(unit_id=1, address=1, quantity=1) == [456]


def test_in_memory_store_setup_rejects_address_outside_configured_table():
    store = InMemoryStore(StoreConfig(holding_register_count=1))

    with pytest.raises(ValueError, match="holding_registers"):
        store.set_holding_register(1, 123)


def test_in_memory_store_setup_rejects_invalid_file_record_reference():
    store = InMemoryStore(StoreConfig())

    with pytest.raises(ValueError, match="file number"):
        store.set_file_record(0, 0, 123)

    with pytest.raises(ValueError, match="file record"):
        store.set_file_record(1, 0x2710, 123)


class PythonStore:
    def __init__(self):
        self.coils = [False] * 16
        self.discrete_inputs = [False] * 16
        self.holding_registers = [0] * 16
        self.input_registers = [0] * 16
        self.holding_registers[0] = 77
        self.input_registers[0] = 88

    def read_coils(self, address, quantity):
        return self.coils[address : address + quantity]

    def write_coil(self, address, value):
        self.coils[address] = value

    def write_coils(self, address, values):
        self.coils[address : address + len(values)] = values

    def read_discrete_inputs(self, address, quantity):
        return self.discrete_inputs[address : address + quantity]

    def read_holding_registers(self, address, quantity):
        if address >= len(self.holding_registers):
            raise IllegalDataAddressError("holding register out of range")
        return self.holding_registers[address : address + quantity]

    def write_register(self, address, value):
        self.holding_registers[address] = value

    def write_registers(self, address, values):
        self.holding_registers[address : address + len(values)] = values

    def read_input_registers(self, address, quantity):
        return self.input_registers[address : address + quantity]


def test_python_data_store_server_round_trip():
    with ModbusServer.start(ServerConfig(), PythonStore()) as server:
        with SyncModbusClient.connect(server.local_addr) as client:
            assert client.read_holding_registers(unit_id=1, address=0, quantity=1) == [77]
            assert client.read_input_registers(unit_id=1, address=0, quantity=1) == [88]

            client.write_single_register(unit_id=1, address=1, value=99)
            assert client.read_holding_registers(unit_id=1, address=1, quantity=1) == [99]


def test_python_data_store_exception_mapping():
    with ModbusServer.start(ServerConfig(), PythonStore()) as server:
        with SyncModbusClient.connect(server.local_addr) as client:
            with pytest.raises(ModbusExceptionError) as exc:
                client.read_holding_registers(unit_id=1, address=100, quantity=1)
            assert exc.value.args[1] == 0x02
