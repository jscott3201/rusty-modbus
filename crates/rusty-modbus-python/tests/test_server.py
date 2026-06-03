"""Tests for Modbus server bindings."""

import socket
import struct

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


def _recv_exact(sock, size):
    data = bytearray()
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise AssertionError("connection closed before full Modbus/TCP frame")
        data.extend(chunk)
    return bytes(data)


def _send_raw_pdu(address, pdu, unit_id=1):
    host, port = address.rsplit(":", 1)
    transaction_id = 0x1234
    request = struct.pack(">HHHB", transaction_id, 0, len(pdu) + 1, unit_id) + pdu

    with socket.create_connection((host, int(port)), timeout=2.0) as sock:
        sock.sendall(request)
        header = _recv_exact(sock, 7)
        rx_transaction_id, protocol_id, length, rx_unit_id = struct.unpack(">HHHB", header)
        assert rx_transaction_id == transaction_id
        assert protocol_id == 0
        assert rx_unit_id == unit_id
        assert length >= 1
        return _recv_exact(sock, length - 1)


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


class PythonOptionalStore(PythonStore):
    def __init__(self):
        super().__init__()
        self.written_file_records = []
        self.diagnostic_calls = []

    def read_file_record(self, file_number, record_number, record_length):
        assert (file_number, record_number, record_length) == (1, 0, 2)
        return [0x1234, 0xABCD]

    def write_file_record(self, file_number, record_number, values):
        self.written_file_records.append((file_number, record_number, list(values)))

    def read_fifo_queue(self, address):
        assert address == 0
        return [0x0102, 0x0304]

    def read_exception_status(self):
        return 0x6D

    def get_comm_event_counter(self):
        return (0x0000, 0x0108)

    def get_comm_event_log(self):
        return (0x0000, 0x0108, 0x0121, [0x20, 0x00])

    def report_server_id(self):
        return b"PY\xff"

    def diagnostic(self, sub_function, data):
        payload = bytes(data)
        self.diagnostic_calls.append((sub_function, payload))
        return [byte ^ 0xFF for byte in payload]


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


def test_python_data_store_optional_callbacks_cover_extended_functions():
    store = PythonOptionalStore()
    read_sub_request = bytes([0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02])
    write_sub_request = bytes(
        [0x06, 0x00, 0x01, 0x00, 0x01, 0x00, 0x02, 0xCA, 0xFE, 0xBA, 0xBE]
    )

    with ModbusServer.start(ServerConfig(), store) as server:
        with SyncModbusClient.connect(server.local_addr) as client:
            assert client.read_fifo_queue(unit_id=1, pointer_address=0) == [0x0102, 0x0304]
            assert client.read_file_record(unit_id=1, sub_request_data=read_sub_request) == bytes(
                [0x05, 0x06, 0x12, 0x34, 0xAB, 0xCD]
            )
            assert (
                client.write_file_record(unit_id=1, sub_request_data=write_sub_request)
                == write_sub_request
            )

        assert _send_raw_pdu(server.local_addr, b"\x07") == b"\x07\x6D"
        assert _send_raw_pdu(server.local_addr, b"\x0B") == b"\x0B\x00\x00\x01\x08"
        assert _send_raw_pdu(server.local_addr, b"\x0C") == bytes(
            [0x0C, 0x08, 0x00, 0x00, 0x01, 0x08, 0x01, 0x21, 0x20, 0x00]
        )
        assert _send_raw_pdu(server.local_addr, b"\x11") == b"\x11\x03PY\xff"
        assert _send_raw_pdu(server.local_addr, b"\x08\x00\x00\x12\x34") == bytes(
            [0x08, 0x00, 0x00, 0xED, 0xCB]
        )

    assert store.written_file_records == [(1, 1, [0xCAFE, 0xBABE])]
    assert store.diagnostic_calls == [(0, b"\x12\x34")]


def test_python_data_store_missing_optional_callbacks_use_spec_defaults():
    with ModbusServer.start(ServerConfig(), PythonStore()) as server:
        with SyncModbusClient.connect(server.local_addr) as client:
            with pytest.raises(ModbusExceptionError) as exc:
                client.read_fifo_queue(unit_id=1, pointer_address=0)
            assert exc.value.args[1] == 0x02

        assert _send_raw_pdu(server.local_addr, b"\x11") == b"\x91\x01"
        assert _send_raw_pdu(server.local_addr, b"\x08\x00\x00\x12\x34") == bytes(
            [0x08, 0x00, 0x00, 0x12, 0x34]
        )
