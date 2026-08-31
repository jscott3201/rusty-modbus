//! Pool configuration types.

use std::net::SocketAddr;
use std::time::Duration;

use rusty_modbus_tcp::TcpConfig;

#[cfg(feature = "client")]
use rusty_modbus_types::{Address, Quantity, UnitId};

#[cfg(feature = "client")]
use crate::error::PriorityProbeConfigError;

/// Connection pool configuration.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum connections in the **non-priority** pool. Default: 64.
    ///
    /// Priority devices have their own per-device budgets
    /// ([`PriorityDevice::max_connections`]) and do **not** count against this.
    pub max_connections: usize,
    /// Priority device entries — healthy/unknown connections to these addresses
    /// are never age- or capacity-evicted. Known-adverse idle transports are retired.
    pub priority_devices: Vec<PriorityDevice>,
    /// Pre-connect to priority devices at pool creation time. Default: `true`.
    pub pre_connect: bool,
    /// Maintain at least one idle TCP connection per distinct configured priority
    /// address whenever its per-device capacity and connectivity permit. Default:
    /// `false`.
    ///
    /// Enabling this also starts the initial priority warm-up when
    /// [`Self::pre_connect`] is `false`. The standing task uses the first matching
    /// [`PriorityDevice`] entry's capacity, reconnects with [`Self::backoff`], and
    /// uses [`Self::health_check_interval`] as a safety-only reevaluation fallback.
    /// It performs no active liveness probe.
    ///
    /// Adding this public field is source-breaking for exhaustive `PoolConfig`
    /// struct literals. Callers should set it explicitly or include
    /// `..PoolConfig::default()` so future configuration fields remain compatible.
    pub priority_replenishment: bool,
    /// Idle timeout before a non-priority connection is eligible for eviction. Default: 300s.
    pub idle_timeout: Duration,
    /// Interval for passive idle validation and non-priority age eviction. Default: 60s.
    pub health_check_interval: Duration,
    /// Priority warm-up and replenishment retry backoff configuration.
    pub backoff: BackoffConfig,
    /// Underlying TCP transport configuration.
    pub tcp_config: TcpConfig,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 64,
            priority_devices: Vec::new(),
            pre_connect: true,
            priority_replenishment: false,
            idle_timeout: Duration::from_mins(5),
            health_check_interval: Duration::from_mins(1),
            backoff: BackoffConfig::default(),
            tcp_config: TcpConfig::default(),
        }
    }
}

/// A priority device entry — healthy/unknown connections to this address are never
/// age- or capacity-evicted; known-adverse idle transports are retired.
#[derive(Debug, Clone)]
pub struct PriorityDevice {
    /// Device socket address.
    pub addr: SocketAddr,
    /// Maximum connections to this device.
    pub max_connections: usize,
    /// Optional read-only active probe configuration.
    ///
    /// Probes are disabled by default and are available only with the `client`
    /// feature. Adding this feature-gated public field is source-breaking for
    /// client-enabled exhaustive `PriorityDevice` literals; set it to `None` to
    /// preserve passive-only behavior.
    #[cfg(feature = "client")]
    pub probe: Option<PriorityProbeConfig>,
}

impl PriorityDevice {
    /// Construct a priority device with active probing disabled.
    ///
    /// This constructor has the same signature with and without the `client`
    /// feature, so callers that do not need a probe can avoid feature-dependent
    /// exhaustive struct literals.
    #[must_use]
    pub const fn new(addr: SocketAddr, max_connections: usize) -> Self {
        Self {
            addr,
            max_connections,
            #[cfg(feature = "client")]
            probe: None,
        }
    }
}

/// Read-only Modbus operation used by a configured priority-device probe.
#[cfg(feature = "client")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PriorityProbeOperation {
    /// Read coils (FC01).
    ReadCoils,
    /// Read discrete inputs (FC02).
    ReadDiscreteInputs,
    /// Read holding registers (FC03).
    ReadHoldingRegisters,
    /// Read input registers (FC04).
    ReadInputRegisters,
}

#[cfg(feature = "client")]
impl PriorityProbeOperation {
    /// Stable bounded label used by probe observability.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadCoils => "read_coils",
            Self::ReadDiscreteInputs => "read_discrete_inputs",
            Self::ReadHoldingRegisters => "read_holding_registers",
            Self::ReadInputRegisters => "read_input_registers",
        }
    }

    const fn maximum_quantity(self) -> u16 {
        match self {
            Self::ReadCoils | Self::ReadDiscreteInputs => 2_000,
            Self::ReadHoldingRegisters | Self::ReadInputRegisters => 125,
        }
    }
}

#[cfg(feature = "client")]
impl std::fmt::Display for PriorityProbeOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Validated configuration for one periodic priority-device read probe.
///
/// Construction accepts only non-broadcast Modbus slave Unit IDs or the direct
/// TCP Unit ID, protocol-valid FC01-FC04 quantities and address spans, and
/// nonzero scheduling durations.
#[cfg(feature = "client")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityProbeConfig {
    operation: PriorityProbeOperation,
    unit_id: UnitId,
    address: Address,
    quantity: Quantity,
    interval: Duration,
    timeout: Duration,
}

#[cfg(feature = "client")]
impl PriorityProbeConfig {
    /// Validate and construct a read-only priority-device probe.
    ///
    /// # Errors
    ///
    /// Returns [`PriorityProbeConfigError`] when the Unit ID, quantity, address
    /// span, interval, or timeout is outside the supported bounds.
    pub fn new(
        operation: PriorityProbeOperation,
        unit_id: UnitId,
        address: Address,
        quantity: Quantity,
        interval: Duration,
        timeout: Duration,
    ) -> Result<Self, PriorityProbeConfigError> {
        if !unit_id.is_valid_slave() && !unit_id.is_tcp_device() {
            return Err(PriorityProbeConfigError::InvalidUnitId(unit_id.0));
        }

        let maximum = operation.maximum_quantity();
        if !(1..=maximum).contains(&quantity.0) {
            return Err(PriorityProbeConfigError::InvalidQuantity {
                operation,
                quantity: quantity.0,
                maximum,
            });
        }

        if u32::from(address.0) + u32::from(quantity.0) > 0x1_0000 {
            return Err(PriorityProbeConfigError::AddressSpanExceeded {
                address: address.0,
                quantity: quantity.0,
            });
        }
        if interval.is_zero() {
            return Err(PriorityProbeConfigError::ZeroInterval);
        }
        if timeout.is_zero() {
            return Err(PriorityProbeConfigError::ZeroTimeout);
        }

        Ok(Self {
            operation,
            unit_id,
            address,
            quantity,
            interval,
            timeout,
        })
    }

    /// Configured read operation.
    #[must_use]
    pub const fn operation(&self) -> PriorityProbeOperation {
        self.operation
    }

    /// Configured Modbus Unit ID.
    #[must_use]
    pub const fn unit_id(&self) -> UnitId {
        self.unit_id
    }

    /// Zero-based Modbus data address.
    #[must_use]
    pub const fn address(&self) -> Address {
        self.address
    }

    /// Number of bits or registers read by each probe.
    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }

    /// Delay between completed or skipped due attempts.
    #[must_use]
    pub const fn interval(&self) -> Duration {
        self.interval
    }

    /// Per-operation timeout and client shutdown timeout.
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// Exponential backoff configuration for reconnection attempts.
#[derive(Debug, Clone)]
pub struct BackoffConfig {
    /// Initial retry delay. Default: 100ms.
    pub initial_delay: Duration,
    /// Maximum retry delay. Default: 30s.
    pub max_delay: Duration,
    /// Multiplier applied on each retry. Default: 2.0.
    pub multiplier: f64,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
        }
    }
}

#[cfg(all(test, feature = "client"))]
mod tests {
    use super::*;

    const INTERVAL: Duration = Duration::from_secs(30);
    const TIMEOUT: Duration = Duration::from_secs(2);

    fn probe(
        operation: PriorityProbeOperation,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> Result<PriorityProbeConfig, PriorityProbeConfigError> {
        PriorityProbeConfig::new(
            operation,
            UnitId(unit_id),
            Address(address),
            Quantity(quantity),
            INTERVAL,
            TIMEOUT,
        )
    }

    #[test]
    fn priority_device_constructor_and_explicit_literal_default_probe_off() {
        let addr = "127.0.0.1:502".parse().unwrap();
        let constructed = PriorityDevice::new(addr, 2);
        let explicit = PriorityDevice {
            addr,
            max_connections: 2,
            probe: None,
        };

        assert!(constructed.probe.is_none());
        assert!(explicit.probe.is_none());
    }

    #[test]
    fn operation_labels_and_quantity_bounds_are_stable() {
        let cases = [
            (PriorityProbeOperation::ReadCoils, "read_coils", 2_000),
            (
                PriorityProbeOperation::ReadDiscreteInputs,
                "read_discrete_inputs",
                2_000,
            ),
            (
                PriorityProbeOperation::ReadHoldingRegisters,
                "read_holding_registers",
                125,
            ),
            (
                PriorityProbeOperation::ReadInputRegisters,
                "read_input_registers",
                125,
            ),
        ];

        for (operation, label, maximum) in cases {
            assert_eq!(operation.as_str(), label);
            assert_eq!(operation.to_string(), label);
            assert!(probe(operation, 1, 0, 1).is_ok());
            assert!(probe(operation, 1, 0, maximum).is_ok());
            assert_eq!(
                probe(operation, 1, 0, 0),
                Err(PriorityProbeConfigError::InvalidQuantity {
                    operation,
                    quantity: 0,
                    maximum,
                })
            );
            assert_eq!(
                probe(operation, 1, 0, maximum + 1),
                Err(PriorityProbeConfigError::InvalidQuantity {
                    operation,
                    quantity: maximum + 1,
                    maximum,
                })
            );
        }
    }

    #[test]
    fn unit_id_and_address_span_boundaries_are_enforced() {
        for unit_id in [1, 247, 255] {
            assert!(probe(PriorityProbeOperation::ReadCoils, unit_id, 0, 1).is_ok());
        }
        for unit_id in [0, 248, 254] {
            let error = probe(PriorityProbeOperation::ReadCoils, unit_id, 0, 1).unwrap_err();
            assert_eq!(error, PriorityProbeConfigError::InvalidUnitId(unit_id));
            assert!(error.to_string().contains(&unit_id.to_string()));
        }

        let edge = probe(PriorityProbeOperation::ReadHoldingRegisters, 1, 0xffff, 1).unwrap();
        assert_eq!(edge.address(), Address(0xffff));
        assert_eq!(edge.quantity(), Quantity(1));
        assert_eq!(
            probe(PriorityProbeOperation::ReadHoldingRegisters, 1, 0xffff, 2),
            Err(PriorityProbeConfigError::AddressSpanExceeded {
                address: 0xffff,
                quantity: 2,
            })
        );
        assert!(probe(PriorityProbeOperation::ReadHoldingRegisters, 1, 0xff83, 125).is_ok());
    }

    #[test]
    fn durations_must_be_nonzero_and_getters_preserve_validated_values() {
        let operation = PriorityProbeOperation::ReadInputRegisters;
        let zero_interval = PriorityProbeConfig::new(
            operation,
            UnitId(255),
            Address(42),
            Quantity(3),
            Duration::ZERO,
            TIMEOUT,
        );
        assert_eq!(zero_interval, Err(PriorityProbeConfigError::ZeroInterval));

        let zero_timeout = PriorityProbeConfig::new(
            operation,
            UnitId(255),
            Address(42),
            Quantity(3),
            INTERVAL,
            Duration::ZERO,
        );
        assert_eq!(zero_timeout, Err(PriorityProbeConfigError::ZeroTimeout));

        let config = probe(operation, 255, 42, 3).unwrap();
        assert_eq!(config.operation(), operation);
        assert_eq!(config.unit_id(), UnitId(255));
        assert_eq!(config.address(), Address(42));
        assert_eq!(config.quantity(), Quantity(3));
        assert_eq!(config.interval(), INTERVAL);
        assert_eq!(config.timeout(), TIMEOUT);
    }
}
