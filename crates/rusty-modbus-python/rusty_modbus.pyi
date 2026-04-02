"""Type stubs for the rusty_modbus Python module."""

from __future__ import annotations

from typing import Coroutine, Sequence

# -- Exceptions ---------------------------------------------------------------

class ModbusError(Exception):
    """Base exception for all rusty_modbus errors."""
    ...

class TimeoutError(ModbusError):
    """Request timed out."""
    ...

class ConnectionError(ModbusError):
    """Transport-level connection failure."""
    ...

class ModbusExceptionError(ModbusError):
    """Server returned a Modbus exception PDU."""
    ...

class RetryError(ModbusError):
    """All retry attempts exhausted."""
    ...

# -- Configuration ------------------------------------------------------------

class ClientConfig:
    """Client connection configuration."""

    unit_id: int
    timeout_secs: float
    max_in_flight: int
    retry: RetryConfig | None

    def __init__(
        self,
        unit_id: int = 255,
        timeout_secs: float = 5.0,
        max_in_flight: int = 16,
        retry: RetryConfig | None = None,
    ) -> None: ...
    def __repr__(self) -> str: ...

class RetryConfig:
    """Retry configuration for transient failures."""

    max_retries: int
    retry_delay_ms: int

    def __init__(
        self,
        max_retries: int = 3,
        retry_delay_ms: int = 100,
    ) -> None: ...
    def __repr__(self) -> str: ...

class TlsConfig:
    """TLS mutual authentication configuration."""

    ca_cert: str
    client_cert: str
    client_key: str
    timeout_secs: float

    def __init__(
        self,
        ca_cert: str,
        client_cert: str,
        client_key: str,
        timeout_secs: float = 5.0,
    ) -> None: ...
    def __repr__(self) -> str: ...

# -- Types --------------------------------------------------------------------

class DeviceIdentification:
    """Device identification returned by FC 0x2B (MEI 0x0E)."""

    vendor_name: str | None
    product_code: str | None
    major_minor_revision: str | None

    def __repr__(self) -> str: ...

# -- Async Client -------------------------------------------------------------

class ModbusClient:
    """Async Modbus client for use with Python asyncio.

    Supports TCP and TLS transports, the full set of Modbus data methods,
    and the async context-manager protocol (``async with``).
    """

    @staticmethod
    def connect(
        address: str,
        config: ClientConfig | None = None,
    ) -> Coroutine[None, None, ModbusClient]:
        """Connect to a Modbus/TCP server."""
        ...

    @staticmethod
    def connect_tls(
        address: str,
        tls: TlsConfig,
        config: ClientConfig | None = None,
    ) -> Coroutine[None, None, ModbusClient]:
        """Connect to a Modbus/TCP Security (TLS) server."""
        ...

    @property
    def is_connected(self) -> bool:
        """Whether the client is currently connected."""
        ...

    def shutdown(self) -> Coroutine[None, None, None]:
        """Gracefully shut down the client."""
        ...

    async def __aenter__(self) -> ModbusClient: ...
    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: object | None,
    ) -> bool: ...

    # -- Register methods -----------------------------------------------------

    def read_holding_registers(
        self, unit_id: int, address: int, quantity: int
    ) -> Coroutine[None, None, list[int]]:
        """Read holding registers (FC 0x03)."""
        ...

    def read_input_registers(
        self, unit_id: int, address: int, quantity: int
    ) -> Coroutine[None, None, list[int]]:
        """Read input registers (FC 0x04)."""
        ...

    def write_single_register(
        self, unit_id: int, address: int, value: int
    ) -> Coroutine[None, None, None]:
        """Write a single register (FC 0x06)."""
        ...

    def write_multiple_registers(
        self, unit_id: int, address: int, values: Sequence[int]
    ) -> Coroutine[None, None, None]:
        """Write multiple registers (FC 0x10)."""
        ...

    def mask_write_register(
        self, unit_id: int, address: int, and_mask: int, or_mask: int
    ) -> Coroutine[None, None, None]:
        """Mask write register (FC 0x16)."""
        ...

    def read_write_multiple_registers(
        self,
        unit_id: int,
        read_address: int,
        read_quantity: int,
        write_address: int,
        write_values: Sequence[int],
    ) -> Coroutine[None, None, list[int]]:
        """Read and write multiple registers (FC 0x17)."""
        ...

    # -- Coil methods ---------------------------------------------------------

    def read_coils(
        self, unit_id: int, address: int, quantity: int
    ) -> Coroutine[None, None, list[bool]]:
        """Read coils (FC 0x01)."""
        ...

    def read_discrete_inputs(
        self, unit_id: int, address: int, quantity: int
    ) -> Coroutine[None, None, list[bool]]:
        """Read discrete inputs (FC 0x02)."""
        ...

    def write_single_coil(
        self, unit_id: int, address: int, value: bool
    ) -> Coroutine[None, None, None]:
        """Write a single coil (FC 0x05)."""
        ...

    def write_multiple_coils(
        self, unit_id: int, address: int, values: Sequence[bool]
    ) -> Coroutine[None, None, None]:
        """Write multiple coils (FC 0x0F)."""
        ...

    # -- FIFO -----------------------------------------------------------------

    def read_fifo_queue(
        self, unit_id: int, pointer_address: int
    ) -> Coroutine[None, None, list[int]]:
        """Read FIFO queue (FC 0x18)."""
        ...

    # -- File records ---------------------------------------------------------

    def read_file_record(
        self, unit_id: int, sub_request_data: bytes
    ) -> Coroutine[None, None, bytes]:
        """Read file record (FC 0x14)."""
        ...

    def write_file_record(
        self, unit_id: int, sub_request_data: bytes
    ) -> Coroutine[None, None, bytes]:
        """Write file record (FC 0x15)."""
        ...

    # -- Device identification ------------------------------------------------

    def read_device_identification(
        self, unit_id: int
    ) -> Coroutine[None, None, DeviceIdentification]:
        """Read device identification (FC 0x2B / MEI 0x0E)."""
        ...

# -- Sync Client --------------------------------------------------------------

class SyncModbusClient:
    """Blocking Modbus client.

    Identical API to ``ModbusClient`` without async/await. Owns a
    tokio runtime internally; each method blocks until the operation
    completes.
    """

    @staticmethod
    def connect(
        address: str,
        config: ClientConfig | None = None,
    ) -> SyncModbusClient:
        """Connect to a Modbus/TCP server (blocking)."""
        ...

    @staticmethod
    def connect_tls(
        address: str,
        tls: TlsConfig,
        config: ClientConfig | None = None,
    ) -> SyncModbusClient:
        """Connect to a Modbus/TCP Security (TLS) server (blocking)."""
        ...

    @property
    def is_connected(self) -> bool:
        """Whether the client is currently connected."""
        ...

    def shutdown(self) -> None:
        """Gracefully shut down the client."""
        ...

    def __enter__(self) -> SyncModbusClient: ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: object | None,
    ) -> None: ...

    # -- Register methods -----------------------------------------------------

    def read_holding_registers(
        self, unit_id: int, address: int, quantity: int
    ) -> list[int]:
        """Read holding registers (FC 0x03)."""
        ...

    def read_input_registers(
        self, unit_id: int, address: int, quantity: int
    ) -> list[int]:
        """Read input registers (FC 0x04)."""
        ...

    def write_single_register(
        self, unit_id: int, address: int, value: int
    ) -> None:
        """Write a single register (FC 0x06)."""
        ...

    def write_multiple_registers(
        self, unit_id: int, address: int, values: Sequence[int]
    ) -> None:
        """Write multiple registers (FC 0x10)."""
        ...

    def mask_write_register(
        self, unit_id: int, address: int, and_mask: int, or_mask: int
    ) -> None:
        """Mask write register (FC 0x16)."""
        ...

    def read_write_multiple_registers(
        self,
        unit_id: int,
        read_address: int,
        read_quantity: int,
        write_address: int,
        write_values: Sequence[int],
    ) -> list[int]:
        """Read and write multiple registers (FC 0x17)."""
        ...

    # -- Coil methods ---------------------------------------------------------

    def read_coils(
        self, unit_id: int, address: int, quantity: int
    ) -> list[bool]:
        """Read coils (FC 0x01)."""
        ...

    def read_discrete_inputs(
        self, unit_id: int, address: int, quantity: int
    ) -> list[bool]:
        """Read discrete inputs (FC 0x02)."""
        ...

    def write_single_coil(
        self, unit_id: int, address: int, value: bool
    ) -> None:
        """Write a single coil (FC 0x05)."""
        ...

    def write_multiple_coils(
        self, unit_id: int, address: int, values: Sequence[bool]
    ) -> None:
        """Write multiple coils (FC 0x0F)."""
        ...

    # -- FIFO -----------------------------------------------------------------

    def read_fifo_queue(
        self, unit_id: int, pointer_address: int
    ) -> list[int]:
        """Read FIFO queue (FC 0x18)."""
        ...

    # -- File records ---------------------------------------------------------

    def read_file_record(
        self, unit_id: int, sub_request_data: bytes
    ) -> bytes:
        """Read file record (FC 0x14)."""
        ...

    def write_file_record(
        self, unit_id: int, sub_request_data: bytes
    ) -> bytes:
        """Write file record (FC 0x15)."""
        ...

    # -- Device identification ------------------------------------------------

    def read_device_identification(
        self, unit_id: int
    ) -> DeviceIdentification:
        """Read device identification (FC 0x2B / MEI 0x0E)."""
        ...
