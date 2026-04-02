"""Full-stack integration tests using an embedded Modbus server."""
import pytest
from rusty_modbus import (
    ModbusClient,
    SyncModbusClient,
    _start_test_server,
)

# Start server once for all tests
_server_addr = None


def get_server_addr():
    global _server_addr
    if _server_addr is None:
        _server_addr = _start_test_server()
    return _server_addr


class TestSyncIntegration:
    def test_read_holding_registers(self):
        addr = get_server_addr()
        with SyncModbusClient.connect(addr) as client:
            regs = client.read_holding_registers(unit_id=1, address=0, quantity=2)
            assert regs == [1, 2]

    def test_read_input_registers(self):
        addr = get_server_addr()
        with SyncModbusClient.connect(addr) as client:
            regs = client.read_input_registers(unit_id=1, address=0, quantity=2)
            assert regs == [1, 2]

    def test_write_single_register(self):
        addr = get_server_addr()
        with SyncModbusClient.connect(addr) as client:
            client.write_single_register(unit_id=1, address=0, value=0x1234)

    def test_write_multiple_registers(self):
        addr = get_server_addr()
        with SyncModbusClient.connect(addr) as client:
            client.write_multiple_registers(unit_id=1, address=0, values=[100, 200])

    def test_mask_write_register(self):
        addr = get_server_addr()
        with SyncModbusClient.connect(addr) as client:
            client.mask_write_register(unit_id=1, address=0, and_mask=0xFF00, or_mask=0x00FF)

    def test_read_write_multiple_registers(self):
        addr = get_server_addr()
        with SyncModbusClient.connect(addr) as client:
            regs = client.read_write_multiple_registers(
                unit_id=1, read_address=0, read_quantity=2,
                write_address=10, write_values=[100, 200],
            )
            assert regs == [1, 2]

    def test_read_coils(self):
        addr = get_server_addr()
        with SyncModbusClient.connect(addr) as client:
            coils = client.read_coils(unit_id=1, address=0, quantity=3)
            assert coils == [True, False, True]

    def test_write_single_coil(self):
        addr = get_server_addr()
        with SyncModbusClient.connect(addr) as client:
            client.write_single_coil(unit_id=1, address=0, value=True)

    def test_write_multiple_coils(self):
        addr = get_server_addr()
        with SyncModbusClient.connect(addr) as client:
            client.write_multiple_coils(unit_id=1, address=0, values=[True, False])

    def test_read_fifo_queue(self):
        addr = get_server_addr()
        with SyncModbusClient.connect(addr) as client:
            values = client.read_fifo_queue(unit_id=1, pointer_address=0)
            assert values == [10, 11]

    def test_context_manager(self):
        addr = get_server_addr()
        with SyncModbusClient.connect(addr) as client:
            assert client.is_connected is True

    def test_is_connected(self):
        addr = get_server_addr()
        client = SyncModbusClient.connect(addr)
        assert client.is_connected is True
        client.shutdown()


class TestAsyncIntegration:
    @pytest.mark.asyncio
    async def test_read_holding_registers(self):
        addr = get_server_addr()
        async with await ModbusClient.connect(addr) as client:
            regs = await client.read_holding_registers(unit_id=1, address=0, quantity=2)
            assert regs == [1, 2]

    @pytest.mark.asyncio
    async def test_write_single_register(self):
        addr = get_server_addr()
        async with await ModbusClient.connect(addr) as client:
            await client.write_single_register(unit_id=1, address=0, value=42)

    @pytest.mark.asyncio
    async def test_read_coils(self):
        addr = get_server_addr()
        async with await ModbusClient.connect(addr) as client:
            coils = await client.read_coils(unit_id=1, address=0, quantity=3)
            assert coils == [True, False, True]

    @pytest.mark.asyncio
    async def test_mask_write_register(self):
        addr = get_server_addr()
        async with await ModbusClient.connect(addr) as client:
            await client.mask_write_register(
                unit_id=1, address=0, and_mask=0xFF00, or_mask=0x00FF,
            )

    @pytest.mark.asyncio
    async def test_read_fifo_queue(self):
        addr = get_server_addr()
        async with await ModbusClient.connect(addr) as client:
            values = await client.read_fifo_queue(unit_id=1, pointer_address=0)
            assert values == [10, 11]

    @pytest.mark.asyncio
    async def test_async_context_manager(self):
        addr = get_server_addr()
        async with await ModbusClient.connect(addr) as client:
            regs = await client.read_holding_registers(unit_id=1, address=0, quantity=2)
            assert regs == [1, 2]

    @pytest.mark.asyncio
    async def test_is_connected(self):
        addr = get_server_addr()
        client = await ModbusClient.connect(addr)
        assert client.is_connected is True
        await client.shutdown()
