//! RTU serial transport configuration.

use std::time::Duration;

/// Number of data bits per character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataBits {
    /// Five data bits.
    Five,
    /// Six data bits.
    Six,
    /// Seven data bits.
    Seven,
    /// Eight data bits.
    Eight,
}

/// Parity checking mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    /// No parity bit.
    None,
    /// Even parity.
    Even,
    /// Odd parity.
    Odd,
}

/// Number of stop bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopBits {
    /// One stop bit.
    One,
    /// Two stop bits.
    Two,
}

/// RTU serial port configuration.
///
/// Default values follow the Modbus serial specification: 9600 baud, 8 data
/// bits, no parity, one stop bit, and a 1-second response timeout.
#[derive(Debug, Clone)]
pub struct RtuConfig {
    /// Baud rate in bits per second. Default: 9600.
    pub baud_rate: u32,
    /// Number of data bits per character. Default: `Eight`.
    pub data_bits: DataBits,
    /// Parity mode. Default: `None`.
    pub parity: Parity,
    /// Number of stop bits. Default: `One` (spec recommends `Two` when parity is `None`).
    pub stop_bits: StopBits,
    /// Maximum time to wait for a response. Default: 1 second.
    pub response_timeout: Duration,
}

impl Default for RtuConfig {
    fn default() -> Self {
        Self {
            baud_rate: 9600,
            data_bits: DataBits::Eight,
            parity: Parity::None,
            stop_bits: StopBits::One,
            response_timeout: Duration::from_secs(1),
        }
    }
}

/// Bits per character in the RTU framing: start(1) + data(8) + parity(0/1) + stop(1/2).
/// The Modbus spec uses 11 bits per character for timing calculations.
const BITS_PER_CHARACTER: f64 = 11.0;

/// Fixed inter-frame delay for baud rates above 19200 (per Modbus spec).
const FIXED_INTERFRAME_DELAY: Duration = Duration::from_micros(1750);

/// Maximum baud rate that uses the calculated (character-time-based) inter-frame delay.
const CALCULATED_DELAY_MAX_BAUD: u32 = 19200;

impl RtuConfig {
    /// Compute the inter-frame delay (silent interval) per the Modbus RTU spec.
    ///
    /// At baud rates of 19200 or below the delay is 3.5 character times,
    /// where one character time is `11 bits / baud_rate` seconds. At baud
    /// rates above 19200 the spec mandates a fixed 1.75 ms delay.
    #[must_use]
    pub fn interframe_delay(&self) -> Duration {
        if self.baud_rate <= CALCULATED_DELAY_MAX_BAUD {
            let char_time_secs = BITS_PER_CHARACTER / f64::from(self.baud_rate);
            Duration::from_secs_f64(3.5 * char_time_secs)
        } else {
            FIXED_INTERFRAME_DELAY
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = RtuConfig::default();
        assert_eq!(cfg.baud_rate, 9600);
        assert_eq!(cfg.data_bits, DataBits::Eight);
        assert_eq!(cfg.parity, Parity::None);
        assert_eq!(cfg.stop_bits, StopBits::One);
        assert_eq!(cfg.response_timeout, Duration::from_secs(1));
    }

    #[test]
    fn interframe_delay_9600() {
        let cfg = RtuConfig::default();
        let delay = cfg.interframe_delay();
        // 3.5 * 11 / 9600 ≈ 4010 µs
        let actual_us = delay.as_micros();
        assert!((4000..=4020).contains(&actual_us));
    }

    #[test]
    fn interframe_delay_19200() {
        let cfg = RtuConfig {
            baud_rate: 19200,
            ..Default::default()
        };
        let delay = cfg.interframe_delay();
        // 3.5 * 11 / 19200 ≈ 2.005 ms — still calculated
        assert!((1900..2100).contains(&delay.as_micros()));
    }

    #[test]
    fn interframe_delay_above_19200_is_fixed() {
        let cfg = RtuConfig {
            baud_rate: 115_200,
            ..Default::default()
        };
        assert_eq!(cfg.interframe_delay(), Duration::from_micros(1750));
    }
}
