//! Physical RTU Unit Identifier address classes.

use std::fmt;

/// Direction in which a physical RTU Unit Identifier is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtuUnitIdRole {
    /// Destination of a client request; zero is the broadcast address.
    ClientDestination,
    /// Source of a responder frame; zero cannot identify a responder.
    ResponderSource,
}

impl fmt::Display for RtuUnitIdRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClientDestination => formatter.write_str("client destination (0..=247)"),
            Self::ResponderSource => formatter.write_str("responder source (1..=247)"),
        }
    }
}

impl RtuUnitIdRole {
    /// Validate a Unit Identifier for this address role.
    ///
    /// Client destinations accept 0 through 247. Responder sources accept 1
    /// through 247. This check classifies addresses only; it does not correlate
    /// a response with an expected peer or restrict which operations broadcast.
    ///
    /// # Errors
    ///
    /// Returns [`RtuUnitIdError`] when `unit_id` is outside the role's range.
    pub const fn validate(self, unit_id: u8) -> Result<(), RtuUnitIdError> {
        match self {
            Self::ClientDestination if unit_id <= 247 => Ok(()),
            Self::ResponderSource if unit_id >= 1 && unit_id <= 247 => Ok(()),
            role => Err(RtuUnitIdError { unit_id, role }),
        }
    }
}

/// A Unit Identifier outside the allowed physical RTU address class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("RTU unit ID {unit_id} is invalid for {role}")]
pub struct RtuUnitIdError {
    unit_id: u8,
    role: RtuUnitIdRole,
}

impl RtuUnitIdError {
    /// Return the rejected Unit Identifier.
    #[must_use]
    pub const fn unit_id(&self) -> u8 {
        self.unit_id
    }

    /// Return the address role used for validation.
    #[must_use]
    pub const fn role(&self) -> RtuUnitIdRole {
        self.role
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_destination_boundaries() {
        for unit_id in [0, 1, 247] {
            assert_eq!(RtuUnitIdRole::ClientDestination.validate(unit_id), Ok(()));
        }
        for unit_id in [248, 255] {
            let error = RtuUnitIdRole::ClientDestination
                .validate(unit_id)
                .unwrap_err();
            assert_eq!(error.unit_id(), unit_id);
            assert_eq!(error.role(), RtuUnitIdRole::ClientDestination);
        }
    }

    #[test]
    fn responder_source_boundaries() {
        for unit_id in [1, 247] {
            assert_eq!(RtuUnitIdRole::ResponderSource.validate(unit_id), Ok(()));
        }
        for unit_id in [0, 248, 255] {
            let error = RtuUnitIdRole::ResponderSource
                .validate(unit_id)
                .unwrap_err();
            assert_eq!(error.unit_id(), unit_id);
            assert_eq!(error.role(), RtuUnitIdRole::ResponderSource);
        }
    }
}
