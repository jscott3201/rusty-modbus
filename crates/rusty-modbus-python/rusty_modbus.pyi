"""Type stubs for the rusty_modbus Python module."""

from __future__ import annotations

from collections.abc import Awaitable, Sequence
from typing import Literal, Protocol, final

_ByteLike = bytes | bytearray | Sequence[int]

__all__ = [
    "ModbusError",
    "TimeoutError",
    "ConnectionError",
    "ModbusExceptionError",
    "RetryError",
    "IllegalFunctionError",
    "IllegalDataAddressError",
    "IllegalDataValueError",
    "ServerDeviceFailureError",
    "AcknowledgeError",
    "ServerDeviceBusyError",
    "NegativeAcknowledgeError",
    "MemoryParityError",
    "GatewayPathUnavailableError",
    "GatewayTargetDeviceFailedToRespondError",
    "ClientConfig",
    "TlsConfig",
    "RetryConfig",
    "DeviceIdentification",
    "ModbusClient",
    "SyncModbusClient",
    "ServerConfig",
    "ServerMetrics",
    "StoreConfig",
    "InMemoryStore",
    "ModbusServer",
]

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

class IllegalFunctionError(ModbusError):
    """Server callback error mapped to Modbus exception 0x01."""
    ...

class IllegalDataAddressError(ModbusError):
    """Server callback error mapped to Modbus exception 0x02."""
    ...

class IllegalDataValueError(ModbusError):
    """Server callback error mapped to Modbus exception 0x03."""
    ...

class ServerDeviceFailureError(ModbusError):
    """Server callback error mapped to Modbus exception 0x04."""
    ...

class AcknowledgeError(ModbusError):
    """Server callback error mapped to Modbus exception 0x05."""
    ...

class ServerDeviceBusyError(ModbusError):
    """Server callback error mapped to Modbus exception 0x06."""
    ...

class NegativeAcknowledgeError(ModbusError):
    """Server callback error mapped to Modbus exception 0x07."""
    ...

class MemoryParityError(ModbusError):
    """Server callback error mapped to Modbus exception 0x08."""
    ...

class GatewayPathUnavailableError(ModbusError):
    """Server callback error mapped to Modbus exception 0x0A."""
    ...

class GatewayTargetDeviceFailedToRespondError(ModbusError):
    """Server callback error mapped to Modbus exception 0x0B."""
    ...

# -- Configuration ------------------------------------------------------------

class ClientConfig:
    """Client connection configuration."""

    @property
    def unit_id(self) -> int: ...
    @property
    def timeout_secs(self) -> float: ...
    @property
    def max_in_flight(self) -> int: ...
    @property
    def retry(self) -> RetryConfig | None: ...
    @property
    def shutdown_timeout_secs(self) -> float: ...

    def __init__(
        self,
        unit_id: int = 255,
        timeout_secs: float = 5.0,
        max_in_flight: int = 16,
        retry: RetryConfig | None = None,
        shutdown_timeout_secs: float = 10.0,
    ) -> None: ...
    def __repr__(self) -> str: ...

class RetryConfig:
    """Retry configuration for transient failures."""

    @property
    def max_retries(self) -> int: ...
    @property
    def retry_delay_ms(self) -> int: ...

    def __init__(
        self,
        max_retries: int = 3,
        retry_delay_ms: int = 100,
    ) -> None: ...
    def __repr__(self) -> str: ...

class TlsConfig:
    """TLS mutual authentication configuration."""

    @property
    def ca_cert(self) -> str: ...
    @property
    def client_cert(self) -> str: ...
    @property
    def client_key(self) -> str: ...
    @property
    def timeout_secs(self) -> float: ...

    def __init__(
        self,
        ca_cert: str,
        client_cert: str,
        client_key: str,
        timeout_secs: float = 5.0,
    ) -> None: ...
    def __repr__(self) -> str: ...

class ServerConfig:
    """Modbus/TCP server configuration."""

    @property
    def listen_addr(self) -> str: ...
    @property
    def unit_id(self) -> int: ...
    @property
    def max_connections(self) -> int: ...
    @property
    def max_transactions(self) -> int: ...
    @property
    def shutdown_timeout_secs(self) -> float: ...

    def __init__(
        self,
        listen_addr: str = "127.0.0.1:0",
        unit_id: int = 1,
        max_connections: int = 64,
        max_transactions: int = 16,
        shutdown_timeout_secs: float = 10.0,
    ) -> None: ...
    def __repr__(self) -> str: ...

class StoreConfig:
    """In-memory data-store sizing configuration."""

    @property
    def coil_count(self) -> int: ...
    @property
    def discrete_input_count(self) -> int: ...
    @property
    def holding_register_count(self) -> int: ...
    @property
    def input_register_count(self) -> int: ...

    def __init__(
        self,
        coil_count: int = 65536,
        discrete_input_count: int = 65536,
        holding_register_count: int = 65536,
        input_register_count: int = 65536,
    ) -> None: ...
    def __repr__(self) -> str: ...

@final
class ServerMetrics:
    """Immutable server counter snapshot."""

    @property
    def active_connections(self) -> int: ...
    @property
    def active_requests(self) -> int: ...
    @property
    def accepted_connections(self) -> int: ...
    @property
    def access_denied_connections(self) -> int: ...
    @property
    def connection_limit_rejections(self) -> int: ...
    @property
    def accept_errors(self) -> int: ...

    def __repr__(self) -> str: ...

# -- Types --------------------------------------------------------------------

class DeviceIdentification:
    """Device identification returned by FC 0x2B (MEI 0x0E)."""

    @property
    def vendor_name(self) -> str | None: ...
    @property
    def product_code(self) -> str | None: ...
    @property
    def major_minor_revision(self) -> str | None: ...

    def __repr__(self) -> str: ...

# -- Server -------------------------------------------------------------------

class DataStore(Protocol):
    """Required callbacks for Python-backed synchronous server data stores."""

    def read_coils(self, address: int, quantity: int) -> Sequence[bool]: ...
    def write_coil(self, address: int, value: bool) -> None: ...
    def write_coils(self, address: int, values: Sequence[bool]) -> None: ...
    def read_discrete_inputs(self, address: int, quantity: int) -> Sequence[bool]: ...
    def read_holding_registers(self, address: int, quantity: int) -> Sequence[int]: ...
    def write_register(self, address: int, value: int) -> None: ...
    def write_registers(self, address: int, values: Sequence[int]) -> None: ...
    def read_input_registers(self, address: int, quantity: int) -> Sequence[int]: ...

class AtomicCompoundDataStore(DataStore, Protocol):
    """Optional atomic FC 0x16/0x17 callbacks for Python-backed stores."""

    def atomic_mask_write_register(
        self, address: int, and_mask: int, or_mask: int
    ) -> None: ...
    def atomic_read_write_registers(
        self,
        read_address: int,
        read_quantity: int,
        write_address: int,
        write_values: Sequence[int],
    ) -> Sequence[int]: ...

class FileRecordDataStore(DataStore, Protocol):
    """Optional FC 0x14/0x15 callbacks for Python-backed stores."""

    def read_file_record(
        self, file_number: int, record_number: int, record_length: int
    ) -> Sequence[int]: ...
    def write_file_record(
        self, file_number: int, record_number: int, values: Sequence[int]
    ) -> None: ...

class FifoDataStore(DataStore, Protocol):
    """Optional FC 0x18 callback for Python-backed stores."""

    def read_fifo_queue(self, address: int) -> Sequence[int]: ...

class SerialDiagnosticsDataStore(DataStore, Protocol):
    """Optional FC 0x07/0x08/0x0B/0x0C callbacks for Python-backed stores."""

    def read_exception_status(self) -> int: ...
    def get_comm_event_counter(self) -> tuple[int, int]: ...
    def get_comm_event_log(self) -> tuple[int, int, int, _ByteLike]: ...
    def diagnostic(self, sub_function: int, data: _ByteLike) -> _ByteLike | None: ...

class ServerIdentificationDataStore(DataStore, Protocol):
    """Optional FC 0x11 callback for Python-backed stores."""

    def report_server_id(self) -> _ByteLike: ...

class InMemoryStore:
    """Thread-safe in-memory Modbus data store."""

    def __init__(self, config: StoreConfig | None = None) -> None: ...
    def set_coil(self, address: int, value: bool) -> None: ...
    def set_discrete_input(self, address: int, value: bool) -> None: ...
    def set_holding_register(self, address: int, value: int) -> None: ...
    def set_input_register(self, address: int, value: int) -> None: ...
    def set_file_record(self, file_number: int, record_number: int, value: int) -> None: ...
    def set_fifo_queue(self, address: int, values: Sequence[int]) -> None: ...
    def set_exception_status(self, status: int) -> None: ...
    def set_server_id(self, data: _ByteLike) -> None: ...
    def __repr__(self) -> str: ...

class ModbusServer:
    """Running Modbus/TCP server."""

    @staticmethod
    def start(
        config: ServerConfig | None = None,
        store: InMemoryStore | DataStore | None = None,
    ) -> ModbusServer: ...

    @property
    def local_addr(self) -> str:
        """Local address the server is bound to."""
        ...

    def stop(self) -> Literal["drained", "forced"]:
        """Stop admission and return the stable shutdown outcome."""
        ...

    def metrics(self) -> ServerMetrics:
        """Return an immutable snapshot of server counters."""
        ...

    def __enter__(self) -> ModbusServer: ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: object | None,
    ) -> bool: ...
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
    ) -> Awaitable[ModbusClient]:
        """Connect to a Modbus/TCP server."""
        ...

    @staticmethod
    def connect_tls(
        address: str,
        tls: TlsConfig,
        config: ClientConfig | None = None,
    ) -> Awaitable[ModbusClient]:
        """Connect to a Modbus/TCP Security (TLS) server."""
        ...

    @property
    def is_connected(self) -> bool:
        """Whether the client is currently connected."""
        ...

    def shutdown(self) -> Awaitable[None]:
        """Gracefully shut down the client."""
        ...

    def abort(self) -> None:
        """Immediately cancel client work without waiting."""
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
    ) -> Awaitable[list[int]]:
        """Read holding registers (FC 0x03)."""
        ...

    def read_input_registers(
        self, unit_id: int, address: int, quantity: int
    ) -> Awaitable[list[int]]:
        """Read input registers (FC 0x04)."""
        ...

    def write_single_register(
        self, unit_id: int, address: int, value: int
    ) -> Awaitable[None]:
        """Write a single register (FC 0x06)."""
        ...

    def write_multiple_registers(
        self, unit_id: int, address: int, values: Sequence[int]
    ) -> Awaitable[None]:
        """Write multiple registers (FC 0x10)."""
        ...

    def mask_write_register(
        self, unit_id: int, address: int, and_mask: int, or_mask: int
    ) -> Awaitable[None]:
        """Mask write register (FC 0x16)."""
        ...

    def read_write_multiple_registers(
        self,
        unit_id: int,
        read_address: int,
        read_quantity: int,
        write_address: int,
        write_values: Sequence[int],
    ) -> Awaitable[list[int]]:
        """Read and write multiple registers (FC 0x17)."""
        ...

    # -- Coil methods ---------------------------------------------------------

    def read_coils(
        self, unit_id: int, address: int, quantity: int
    ) -> Awaitable[list[bool]]:
        """Read coils (FC 0x01)."""
        ...

    def read_discrete_inputs(
        self, unit_id: int, address: int, quantity: int
    ) -> Awaitable[list[bool]]:
        """Read discrete inputs (FC 0x02)."""
        ...

    def write_single_coil(
        self, unit_id: int, address: int, value: bool
    ) -> Awaitable[None]:
        """Write a single coil (FC 0x05)."""
        ...

    def write_multiple_coils(
        self, unit_id: int, address: int, values: Sequence[bool]
    ) -> Awaitable[None]:
        """Write multiple coils (FC 0x0F)."""
        ...

    # -- FIFO -----------------------------------------------------------------

    def read_fifo_queue(
        self, unit_id: int, pointer_address: int
    ) -> Awaitable[list[int]]:
        """Read FIFO queue (FC 0x18)."""
        ...

    # -- File records ---------------------------------------------------------

    def read_file_record(
        self, unit_id: int, sub_request_data: _ByteLike
    ) -> Awaitable[bytes]:
        """Read file record (FC 0x14)."""
        ...

    def write_file_record(
        self, unit_id: int, sub_request_data: _ByteLike
    ) -> Awaitable[bytes]:
        """Write file record (FC 0x15)."""
        ...

    # -- Device identification ------------------------------------------------

    def read_device_identification(
        self, unit_id: int
    ) -> Awaitable[DeviceIdentification]:
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

    def abort(self) -> None:
        """Immediately cancel client work without waiting."""
        ...

    def __enter__(self) -> SyncModbusClient: ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: object | None,
    ) -> bool: ...

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
        self, unit_id: int, sub_request_data: _ByteLike
    ) -> bytes:
        """Read file record (FC 0x14)."""
        ...

    def write_file_record(
        self, unit_id: int, sub_request_data: _ByteLike
    ) -> bytes:
        """Write file record (FC 0x15)."""
        ...

    # -- Device identification ------------------------------------------------

    def read_device_identification(
        self, unit_id: int
    ) -> DeviceIdentification:
        """Read device identification (FC 0x2B / MEI 0x0E)."""
        ...
