//! RTU serial transport configuration.

use std::num::NonZeroU32;
use std::time::Duration;

use crate::error::RtuConfigError;

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

/// Character formats accepted by the strict physical RTU profile.
///
/// Every variant occupies 11 bits on the wire: one start bit, eight data bits,
/// and either one parity plus one stop bit or two stop bits without parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtuSerialFormat {
    /// Eight data bits, even parity, and one stop bit (8E1).
    EightEvenOne,
    /// Eight data bits, odd parity, and one stop bit (8O1).
    EightOddOne,
    /// Eight data bits, no parity, and two stop bits (8N2).
    EightNoneTwo,
}

/// Compatibility configuration for an RTU serial port.
///
/// This type preserves the original 9600/8N1 default and permits arbitrary raw
/// character settings. Use [`StrictRtuConfig`] when the configuration must be
/// limited to Modbus character formats and a nonzero baud rate.
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

/// Validated configuration for a physical Modbus RTU serial line.
///
/// Unlike [`RtuConfig`], this type cannot contain a zero baud rate or a
/// character format outside 8E1, 8O1, and 8N2. Use
/// [`SerialTransport::open_strict`](crate::serial::SerialTransport::open_strict)
/// to apply these settings to a serial port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrictRtuConfig {
    baud_rate: NonZeroU32,
    serial_format: RtuSerialFormat,
    response_timeout: Duration,
}

impl StrictRtuConfig {
    /// Construct a strict RTU configuration.
    ///
    /// # Errors
    ///
    /// Returns [`RtuConfigError::ZeroBaudRate`] when `baud_rate` is zero.
    pub fn new(
        baud_rate: u32,
        serial_format: RtuSerialFormat,
        response_timeout: Duration,
    ) -> Result<Self, RtuConfigError> {
        let baud_rate = NonZeroU32::new(baud_rate).ok_or(RtuConfigError::ZeroBaudRate)?;
        Ok(Self {
            baud_rate,
            serial_format,
            response_timeout,
        })
    }

    /// Return the configured baud rate in bits per second.
    #[must_use]
    pub const fn baud_rate(&self) -> u32 {
        self.baud_rate.get()
    }

    /// Return the validated serial character format.
    #[must_use]
    pub const fn serial_format(&self) -> RtuSerialFormat {
        self.serial_format
    }

    /// Return the maximum time to wait for a response.
    #[must_use]
    pub const fn response_timeout(&self) -> Duration {
        self.response_timeout
    }

    /// Resolve this configuration to concrete port settings and RTU timers.
    #[must_use]
    pub fn resolve(&self) -> ResolvedRtuConfig {
        ResolvedRtuConfig::from(self)
    }
}

impl Default for StrictRtuConfig {
    fn default() -> Self {
        Self {
            baud_rate: NonZeroU32::new(19_200).expect("19,200 is nonzero"),
            serial_format: RtuSerialFormat::EightEvenOne,
            response_timeout: Duration::from_secs(1),
        }
    }
}

impl TryFrom<&RtuConfig> for StrictRtuConfig {
    type Error = RtuConfigError;

    fn try_from(config: &RtuConfig) -> Result<Self, Self::Error> {
        if config.baud_rate == 0 {
            return Err(RtuConfigError::ZeroBaudRate);
        }

        let serial_format = match (config.data_bits, config.parity, config.stop_bits) {
            (DataBits::Eight, Parity::Even, StopBits::One) => RtuSerialFormat::EightEvenOne,
            (DataBits::Eight, Parity::Odd, StopBits::One) => RtuSerialFormat::EightOddOne,
            (DataBits::Eight, Parity::None, StopBits::Two) => RtuSerialFormat::EightNoneTwo,
            (data_bits, parity, stop_bits) => {
                return Err(RtuConfigError::InvalidSerialFormat {
                    data_bits,
                    parity,
                    stop_bits,
                });
            }
        };

        Self::new(config.baud_rate, serial_format, config.response_timeout)
    }
}

/// Rule used to derive the resolved RTU t1.5 and t3.5 timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtuTimingMode {
    /// Timers are calculated from 1.5 and 3.5 11-bit character times.
    CharacterCalculated,
    /// Timers use the recommended fixed values for baud rates above 19,200.
    FixedHighSpeedRecommendation,
}

/// Concrete settings and timers derived from a [`StrictRtuConfig`].
///
/// Nanosecond timing values are rounded toward longer intervals. The t1.5 and
/// t3.5 values are calculated independently from the baud rate, so rounding the
/// character time cannot shorten either interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedRtuConfig {
    baud_rate: u32,
    serial_format: RtuSerialFormat,
    data_bits: DataBits,
    parity: Parity,
    stop_bits: StopBits,
    response_timeout: Duration,
    character_time: Duration,
    t1_5: Duration,
    t3_5: Duration,
    timing_mode: RtuTimingMode,
}

impl ResolvedRtuConfig {
    /// Return the baud rate in bits per second.
    #[must_use]
    pub const fn baud_rate(&self) -> u32 {
        self.baud_rate
    }

    /// Return the strict serial format from which these settings were resolved.
    #[must_use]
    pub const fn serial_format(&self) -> RtuSerialFormat {
        self.serial_format
    }

    /// Return the concrete data-bit setting for the serial driver.
    #[must_use]
    pub const fn data_bits(&self) -> DataBits {
        self.data_bits
    }

    /// Return the concrete parity setting for the serial driver.
    #[must_use]
    pub const fn parity(&self) -> Parity {
        self.parity
    }

    /// Return the concrete stop-bit setting for the serial driver.
    #[must_use]
    pub const fn stop_bits(&self) -> StopBits {
        self.stop_bits
    }

    /// Return the maximum time to wait for a response.
    #[must_use]
    pub const fn response_timeout(&self) -> Duration {
        self.response_timeout
    }

    /// Return one 11-bit character time, rounded up to whole nanoseconds.
    #[must_use]
    pub const fn character_time(&self) -> Duration {
        self.character_time
    }

    /// Return t1.5, rounded up to whole nanoseconds when character-calculated.
    #[must_use]
    pub const fn t1_5(&self) -> Duration {
        self.t1_5
    }

    /// Return t3.5, rounded up to whole nanoseconds when character-calculated.
    #[must_use]
    pub const fn t3_5(&self) -> Duration {
        self.t3_5
    }

    /// Return the rule used to select t1.5 and t3.5.
    #[must_use]
    pub const fn timing_mode(&self) -> RtuTimingMode {
        self.timing_mode
    }
}

impl From<&StrictRtuConfig> for ResolvedRtuConfig {
    fn from(config: &StrictRtuConfig) -> Self {
        const CHARACTER_NANOSECOND_NUMERATOR: u64 = 11_000_000_000;
        const T1_5_NANOSECOND_NUMERATOR: u64 = 16_500_000_000;
        const T3_5_NANOSECOND_NUMERATOR: u64 = 38_500_000_000;

        let baud_rate = config.baud_rate();
        let (data_bits, parity, stop_bits) = match config.serial_format() {
            RtuSerialFormat::EightEvenOne => (DataBits::Eight, Parity::Even, StopBits::One),
            RtuSerialFormat::EightOddOne => (DataBits::Eight, Parity::Odd, StopBits::One),
            RtuSerialFormat::EightNoneTwo => (DataBits::Eight, Parity::None, StopBits::Two),
        };
        let character_time =
            duration_from_ratio_ceiling(CHARACTER_NANOSECOND_NUMERATOR, config.baud_rate);
        let (t1_5, t3_5, timing_mode) = if baud_rate <= CALCULATED_DELAY_MAX_BAUD {
            (
                duration_from_ratio_ceiling(T1_5_NANOSECOND_NUMERATOR, config.baud_rate),
                duration_from_ratio_ceiling(T3_5_NANOSECOND_NUMERATOR, config.baud_rate),
                RtuTimingMode::CharacterCalculated,
            )
        } else {
            (
                FIXED_INTERCHARACTER_TIMEOUT,
                FIXED_INTERFRAME_DELAY,
                RtuTimingMode::FixedHighSpeedRecommendation,
            )
        };

        Self {
            baud_rate,
            serial_format: config.serial_format(),
            data_bits,
            parity,
            stop_bits,
            response_timeout: config.response_timeout(),
            character_time,
            t1_5,
            t3_5,
            timing_mode,
        }
    }
}

impl From<StrictRtuConfig> for ResolvedRtuConfig {
    fn from(config: StrictRtuConfig) -> Self {
        Self::from(&config)
    }
}

fn duration_from_ratio_ceiling(nanosecond_numerator: u64, baud_rate: NonZeroU32) -> Duration {
    let nanoseconds = nanosecond_numerator.div_ceil(u64::from(baud_rate.get()));
    Duration::from_nanos(nanoseconds)
}

/// The compatibility timing path assumes 11 bits regardless of its raw fields.
const BITS_PER_CHARACTER: f64 = 11.0;

/// Fixed inter-frame delay recommended for baud rates above 19200.
const FIXED_INTERFRAME_DELAY: Duration = Duration::from_micros(1750);

/// Fixed inter-character timeout recommended for baud rates above 19200.
const FIXED_INTERCHARACTER_TIMEOUT: Duration = Duration::from_micros(750);

/// Maximum baud rate that uses the calculated (character-time-based) inter-frame delay.
const CALCULATED_DELAY_MAX_BAUD: u32 = 19200;

impl RtuConfig {
    /// Compute the inter-frame delay (silent interval) per the Modbus RTU spec.
    ///
    /// At baud rates of 19200 or below the delay is 3.5 character times,
    /// where one character time is `11 bits / baud_rate` seconds. At baud
    /// rates above 19200 the legacy path uses a fixed 1.75 ms delay.
    ///
    /// # Panics
    ///
    /// Panics when `baud_rate` is zero. [`StrictRtuConfig`] rejects zero during
    /// construction.
    #[must_use]
    pub fn interframe_delay(&self) -> Duration {
        if self.baud_rate <= CALCULATED_DELAY_MAX_BAUD {
            let char_time_secs = BITS_PER_CHARACTER / f64::from(self.baud_rate);
            Duration::from_secs_f64(3.5 * char_time_secs)
        } else {
            FIXED_INTERFRAME_DELAY
        }
    }

    /// Compute the inter-character timeout per the Modbus RTU spec.
    ///
    /// At baud rates of 19200 or below the timeout is 1.5 character times,
    /// where one character time is `11 bits / baud_rate` seconds. At baud
    /// rates above 19200 the legacy path uses a fixed 750 us timeout.
    ///
    /// # Panics
    ///
    /// Panics when `baud_rate` is zero. [`StrictRtuConfig`] rejects zero during
    /// construction.
    #[must_use]
    pub fn intercharacter_timeout(&self) -> Duration {
        if self.baud_rate <= CALCULATED_DELAY_MAX_BAUD {
            let char_time_secs = BITS_PER_CHARACTER / f64::from(self.baud_rate);
            Duration::from_secs_f64(1.5 * char_time_secs)
        } else {
            FIXED_INTERCHARACTER_TIMEOUT
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
    fn legacy_default_timing_is_unchanged() {
        let cfg = RtuConfig::default();

        assert_eq!(
            cfg.intercharacter_timeout(),
            Duration::from_nanos(1_718_750)
        );
        assert_eq!(cfg.interframe_delay(), Duration::from_nanos(4_010_417));
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

    #[test]
    fn intercharacter_timeout_9600() {
        let cfg = RtuConfig::default();
        let timeout = cfg.intercharacter_timeout();
        // 1.5 * 11 / 9600 ≈ 1719 µs
        let actual_us = timeout.as_micros();
        assert!((1700..=1730).contains(&actual_us));
    }

    #[test]
    fn intercharacter_timeout_19200() {
        let cfg = RtuConfig {
            baud_rate: 19200,
            ..Default::default()
        };
        let timeout = cfg.intercharacter_timeout();
        // 1.5 * 11 / 19200 ≈ 859 µs — still calculated
        assert!((840..=880).contains(&timeout.as_micros()));
    }

    #[test]
    fn intercharacter_timeout_above_19200_is_fixed() {
        let cfg = RtuConfig {
            baud_rate: 115_200,
            ..Default::default()
        };
        assert_eq!(cfg.intercharacter_timeout(), Duration::from_micros(750));
    }

    #[test]
    fn strict_default_uses_serial_spec_defaults() {
        let cfg = StrictRtuConfig::default();

        assert_eq!(cfg.baud_rate(), 19_200);
        assert_eq!(cfg.serial_format(), RtuSerialFormat::EightEvenOne);
        assert_eq!(cfg.response_timeout(), Duration::from_secs(1));
    }

    #[test]
    fn strict_constructor_accepts_every_strict_format() {
        for format in [
            RtuSerialFormat::EightEvenOne,
            RtuSerialFormat::EightOddOne,
            RtuSerialFormat::EightNoneTwo,
        ] {
            let cfg = StrictRtuConfig::new(9_600, format, Duration::from_millis(250)).unwrap();
            assert_eq!(cfg.baud_rate(), 9_600);
            assert_eq!(cfg.serial_format(), format);
            assert_eq!(cfg.response_timeout(), Duration::from_millis(250));
        }
    }

    #[test]
    fn strict_constructor_rejects_zero_baud() {
        assert_eq!(
            StrictRtuConfig::new(0, RtuSerialFormat::EightEvenOne, Duration::from_secs(1)),
            Err(RtuConfigError::ZeroBaudRate)
        );
    }

    #[test]
    fn raw_conversion_accepts_only_the_three_strict_formats() {
        let accepted = [
            (Parity::Even, StopBits::One, RtuSerialFormat::EightEvenOne),
            (Parity::Odd, StopBits::One, RtuSerialFormat::EightOddOne),
            (Parity::None, StopBits::Two, RtuSerialFormat::EightNoneTwo),
        ];

        for (parity, stop_bits, expected) in accepted {
            let raw = RtuConfig {
                baud_rate: 38_400,
                data_bits: DataBits::Eight,
                parity,
                stop_bits,
                response_timeout: Duration::from_millis(300),
            };
            let strict = StrictRtuConfig::try_from(&raw).unwrap();
            assert_eq!(strict.serial_format(), expected);
            assert_eq!(strict.baud_rate(), raw.baud_rate);
            assert_eq!(strict.response_timeout(), raw.response_timeout);
        }
    }

    #[test]
    fn raw_conversion_rejects_every_other_character_combination() {
        let data_bits = [
            DataBits::Five,
            DataBits::Six,
            DataBits::Seven,
            DataBits::Eight,
        ];
        let parities = [Parity::None, Parity::Even, Parity::Odd];
        let stop_bits = [StopBits::One, StopBits::Two];

        for data_bits in data_bits {
            for parity in parities {
                for stop_bits in stop_bits {
                    let is_valid = matches!(
                        (data_bits, parity, stop_bits),
                        (DataBits::Eight, Parity::Even | Parity::Odd, StopBits::One)
                            | (DataBits::Eight, Parity::None, StopBits::Two)
                    );
                    if is_valid {
                        continue;
                    }
                    let raw = RtuConfig {
                        baud_rate: 9_600,
                        data_bits,
                        parity,
                        stop_bits,
                        response_timeout: Duration::from_secs(1),
                    };

                    assert_eq!(
                        StrictRtuConfig::try_from(&raw),
                        Err(RtuConfigError::InvalidSerialFormat {
                            data_bits,
                            parity,
                            stop_bits,
                        })
                    );
                }
            }
        }
    }

    #[test]
    fn raw_conversion_rejects_zero_before_character_format() {
        let raw = RtuConfig {
            baud_rate: 0,
            data_bits: DataBits::Five,
            parity: Parity::None,
            stop_bits: StopBits::One,
            response_timeout: Duration::from_secs(1),
        };

        assert_eq!(
            StrictRtuConfig::try_from(&raw),
            Err(RtuConfigError::ZeroBaudRate)
        );
    }

    #[test]
    fn strict_formats_resolve_to_concrete_eleven_bit_settings() {
        let cases = [
            (RtuSerialFormat::EightEvenOne, Parity::Even, StopBits::One),
            (RtuSerialFormat::EightOddOne, Parity::Odd, StopBits::One),
            (RtuSerialFormat::EightNoneTwo, Parity::None, StopBits::Two),
        ];

        for (format, parity, stop_bits) in cases {
            let resolved = StrictRtuConfig::new(9_600, format, Duration::from_secs(2))
                .unwrap()
                .resolve();
            assert_eq!(resolved.serial_format(), format);
            assert_eq!(resolved.data_bits(), DataBits::Eight);
            assert_eq!(resolved.parity(), parity);
            assert_eq!(resolved.stop_bits(), stop_bits);
            assert_eq!(resolved.response_timeout(), Duration::from_secs(2));
        }
    }

    #[test]
    fn strict_timing_vectors_are_exact_at_9600_and_19200() {
        let at_9600 =
            StrictRtuConfig::new(9_600, RtuSerialFormat::EightEvenOne, Duration::from_secs(1))
                .unwrap()
                .resolve();
        assert_eq!(at_9600.character_time(), Duration::from_nanos(1_145_834));
        assert_eq!(at_9600.t1_5(), Duration::from_nanos(1_718_750));
        assert_eq!(at_9600.t3_5(), Duration::from_nanos(4_010_417));
        assert_eq!(at_9600.timing_mode(), RtuTimingMode::CharacterCalculated);

        let at_19200 = StrictRtuConfig::default().resolve();
        assert_eq!(at_19200.character_time(), Duration::from_nanos(572_917));
        assert_eq!(at_19200.t1_5(), Duration::from_nanos(859_375));
        assert_eq!(at_19200.t3_5(), Duration::from_nanos(2_005_209));
        assert_eq!(at_19200.timing_mode(), RtuTimingMode::CharacterCalculated);
    }

    #[test]
    fn strict_irregular_baud_rounds_each_interval_independently_upward() {
        let resolved =
            StrictRtuConfig::new(12_345, RtuSerialFormat::EightOddOne, Duration::from_secs(1))
                .unwrap()
                .resolve();

        assert_eq!(resolved.character_time(), Duration::from_nanos(891_050));
        assert_eq!(resolved.t1_5(), Duration::from_nanos(1_336_574));
        assert_eq!(resolved.t3_5(), Duration::from_nanos(3_118_672));
        assert_ne!(resolved.t1_5(), resolved.character_time().mul_f64(1.5));
        assert_ne!(resolved.t3_5(), resolved.character_time().mul_f64(3.5));
    }

    #[test]
    fn strict_high_speed_boundary_uses_recommended_fixed_timers() {
        let resolved = StrictRtuConfig::new(
            19_201,
            RtuSerialFormat::EightNoneTwo,
            Duration::from_secs(1),
        )
        .unwrap()
        .resolve();

        assert_eq!(resolved.character_time(), Duration::from_nanos(572_887));
        assert_eq!(resolved.t1_5(), Duration::from_micros(750));
        assert_eq!(resolved.t3_5(), Duration::from_micros(1_750));
        assert_eq!(
            resolved.timing_mode(),
            RtuTimingMode::FixedHighSpeedRecommendation
        );
    }

    #[test]
    fn strict_maximum_baud_does_not_wrap_or_panic() {
        let resolved = StrictRtuConfig::new(
            u32::MAX,
            RtuSerialFormat::EightEvenOne,
            Duration::from_secs(1),
        )
        .unwrap()
        .resolve();

        assert_eq!(resolved.baud_rate(), u32::MAX);
        assert_eq!(resolved.character_time(), Duration::from_nanos(3));
        assert_eq!(resolved.t1_5(), Duration::from_micros(750));
        assert_eq!(resolved.t3_5(), Duration::from_micros(1_750));
    }
}
