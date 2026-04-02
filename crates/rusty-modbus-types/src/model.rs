//! Newtypes and data model types for type-safe Modbus programming.

use crate::constants::{
    UNIT_ID_BROADCAST, UNIT_ID_MAX_SLAVE, UNIT_ID_MIN_SLAVE, UNIT_ID_TCP_DEVICE,
};

/// Transaction identifier for MBAP pipelining (0..=65535).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TransactionId(pub u16);

/// Modbus unit/slave identifier (0..=255).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct UnitId(pub u8);

impl UnitId {
    /// Returns `true` if this is the broadcast address (0x00).
    #[must_use]
    pub fn is_broadcast(self) -> bool {
        self.0 == UNIT_ID_BROADCAST
    }

    /// Returns `true` if this is the direct TCP device address (0xFF).
    #[must_use]
    pub fn is_tcp_device(self) -> bool {
        self.0 == UNIT_ID_TCP_DEVICE
    }

    /// Returns `true` if this is a valid slave address (1..=247).
    #[must_use]
    pub fn is_valid_slave(self) -> bool {
        (UNIT_ID_MIN_SLAVE..=UNIT_ID_MAX_SLAVE).contains(&self.0)
    }
}

/// Modbus data address (0..=65535). Zero-indexed on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Address(pub u16);

/// Quantity of items (coils or registers) in a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Quantity(pub u16);

/// The four primary Modbus data tables from Spec V1.1b3 §4.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DataTable {
    /// Single-bit, read-only. Provided by I/O system.
    DiscreteInput,
    /// Single-bit, read-write. Alterable by application.
    Coil,
    /// 16-bit word, read-only. Provided by I/O system.
    InputRegister,
    /// 16-bit word, read-write. Alterable by application.
    HoldingRegister,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_id_broadcast() {
        assert!(UnitId(0x00).is_broadcast());
        assert!(!UnitId(0x01).is_broadcast());
        assert!(!UnitId(0xFF).is_broadcast());
    }

    #[test]
    fn unit_id_tcp_device() {
        assert!(UnitId(0xFF).is_tcp_device());
        assert!(!UnitId(0x00).is_tcp_device());
        assert!(!UnitId(0x01).is_tcp_device());
    }

    #[test]
    fn unit_id_valid_slave() {
        assert!(!UnitId(0).is_valid_slave());
        assert!(UnitId(1).is_valid_slave());
        assert!(UnitId(100).is_valid_slave());
        assert!(UnitId(247).is_valid_slave());
        assert!(!UnitId(248).is_valid_slave());
        assert!(!UnitId(255).is_valid_slave());
    }

    #[test]
    fn newtypes_are_ordered() {
        assert!(TransactionId(1) < TransactionId(2));
        assert!(Address(0) < Address(100));
        assert!(Quantity(5) > Quantity(3));
    }

    #[test]
    fn data_table_variants() {
        let tables = [
            DataTable::DiscreteInput,
            DataTable::Coil,
            DataTable::InputRegister,
            DataTable::HoldingRegister,
        ];
        // Ensure they're all distinct.
        for (i, a) in tables.iter().enumerate() {
            for (j, b) in tables.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }
}
