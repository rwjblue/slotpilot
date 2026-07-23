//! Integer-backed band, frequency, power, mode, and UTC-slot values.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Failure parsing or constructing a radio-domain value.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RadioValueError {
    /// A symbolic band name is unsupported.
    #[error("unsupported amateur band")]
    InvalidBand,
    /// A frequency was outside its documented bound.
    #[error("{kind} frequency must be between {minimum} and {maximum} Hz")]
    FrequencyOutOfRange {
        /// Frequency role.
        kind: &'static str,
        /// Inclusive lower bound.
        minimum: u64,
        /// Inclusive upper bound.
        maximum: u64,
    },
    /// Power was zero or above the bounded representation.
    #[error("power must be between 1 and 1,000,000 milliwatts")]
    PowerOutOfRange,
    /// A mode name is not part of the Phase 0 synchronized-mode vocabulary.
    #[error("unsupported operating mode")]
    InvalidMode,
    /// A UTC timestamp was negative.
    #[error("UTC slot start must not precede the Unix epoch")]
    NegativeSlot,
    /// A UTC timestamp was not aligned to the mode's slot duration.
    #[error("UTC slot start is not aligned to its operating mode")]
    MisalignedSlot,
    /// A numeric wire value could not be parsed.
    #[error("invalid integer representation")]
    InvalidInteger,
}

/// Supported amateur bands with stable lowercase wire/display names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Band {
    /// 2,200 metres.
    #[serde(rename = "2200m")]
    B2200m,
    /// 630 metres.
    #[serde(rename = "630m")]
    B630m,
    /// 160 metres.
    #[serde(rename = "160m")]
    B160m,
    /// 80 metres.
    #[serde(rename = "80m")]
    B80m,
    /// 60 metres.
    #[serde(rename = "60m")]
    B60m,
    /// 40 metres.
    #[serde(rename = "40m")]
    B40m,
    /// 30 metres.
    #[serde(rename = "30m")]
    B30m,
    /// 20 metres.
    #[serde(rename = "20m")]
    B20m,
    /// 17 metres.
    #[serde(rename = "17m")]
    B17m,
    /// 15 metres.
    #[serde(rename = "15m")]
    B15m,
    /// 12 metres.
    #[serde(rename = "12m")]
    B12m,
    /// 10 metres.
    #[serde(rename = "10m")]
    B10m,
    /// 6 metres.
    #[serde(rename = "6m")]
    B6m,
    /// 2 metres.
    #[serde(rename = "2m")]
    B2m,
    /// 70 centimetres.
    #[serde(rename = "70cm")]
    B70cm,
}

impl Band {
    /// Returns the stable wire/display name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::B2200m => "2200m",
            Self::B630m => "630m",
            Self::B160m => "160m",
            Self::B80m => "80m",
            Self::B60m => "60m",
            Self::B40m => "40m",
            Self::B30m => "30m",
            Self::B20m => "20m",
            Self::B17m => "17m",
            Self::B15m => "15m",
            Self::B12m => "12m",
            Self::B10m => "10m",
            Self::B6m => "6m",
            Self::B2m => "2m",
            Self::B70cm => "70cm",
        }
    }
}

impl fmt::Display for Band {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Band {
    type Err = RadioValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        [
            Self::B2200m,
            Self::B630m,
            Self::B160m,
            Self::B80m,
            Self::B60m,
            Self::B40m,
            Self::B30m,
            Self::B20m,
            Self::B17m,
            Self::B15m,
            Self::B12m,
            Self::B10m,
            Self::B6m,
            Self::B2m,
            Self::B70cm,
        ]
        .into_iter()
        .find(|band| band.as_str() == value)
        .ok_or(RadioValueError::InvalidBand)
    }
}

macro_rules! frequency {
    ($name:ident, $kind:literal, $minimum:literal, $maximum:literal, $docs:literal) => {
        #[doc = $docs]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(try_from = "u64", into = "u64")]
        pub struct $name(u64);

        impl $name {
            /// Constructs a validated frequency in integer hertz.
            pub fn from_hz(hz: u64) -> Result<Self, RadioValueError> {
                if !($minimum..=$maximum).contains(&hz) {
                    return Err(RadioValueError::FrequencyOutOfRange {
                        kind: $kind,
                        minimum: $minimum,
                        maximum: $maximum,
                    });
                }
                Ok(Self(hz))
            }

            /// Returns the integer frequency in hertz.
            #[must_use]
            pub const fn hz(self) -> u64 {
                self.0
            }
        }

        impl TryFrom<u64> for $name {
            type Error = RadioValueError;

            fn try_from(value: u64) -> Result<Self, Self::Error> {
                Self::from_hz(value)
            }
        }

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl FromStr for $name {
            type Err = RadioValueError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value
                    .parse()
                    .map_err(|_| RadioValueError::InvalidInteger)
                    .and_then(Self::from_hz)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

frequency!(
    DialFrequency,
    "dial",
    100_000,
    300_000_000_000,
    "A dial frequency in integer hertz."
);
frequency!(
    AudioFrequency,
    "audio",
    1,
    50_000,
    "An audio offset in integer hertz."
);

/// Transmitter power in integer milliwatts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct Power(u64);

impl Power {
    /// Constructs bounded nonzero power.
    pub fn from_milliwatts(value: u64) -> Result<Self, RadioValueError> {
        if !(1..=1_000_000).contains(&value) {
            return Err(RadioValueError::PowerOutOfRange);
        }
        Ok(Self(value))
    }

    /// Returns power in milliwatts.
    #[must_use]
    pub const fn milliwatts(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for Power {
    type Error = RadioValueError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::from_milliwatts(value)
    }
}

impl From<Power> for u64 {
    fn from(value: Power) -> Self {
        value.0
    }
}

/// Synchronized operating modes represented in Phase 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OperatingMode {
    /// FT8, with 15-second slots.
    Ft8,
    /// WSPR, with two-minute slots.
    Wspr,
}

impl OperatingMode {
    /// Returns the slot duration in milliseconds.
    #[must_use]
    pub const fn slot_millis(self) -> i64 {
        match self {
            Self::Ft8 => 15_000,
            Self::Wspr => 120_000,
        }
    }
}

impl fmt::Display for OperatingMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Ft8 => "FT8",
            Self::Wspr => "WSPR",
        })
    }
}

impl FromStr for OperatingMode {
    type Err = RadioValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "FT8" => Ok(Self::Ft8),
            "WSPR" => Ok(Self::Wspr),
            _ => Err(RadioValueError::InvalidMode),
        }
    }
}

/// An exact UTC-aligned protocol-slot start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UtcSlot {
    mode: OperatingMode,
    start_unix_millis: i64,
}

impl UtcSlot {
    /// Constructs a slot after checking epoch and mode alignment.
    pub fn new(mode: OperatingMode, start_unix_millis: i64) -> Result<Self, RadioValueError> {
        if start_unix_millis < 0 {
            return Err(RadioValueError::NegativeSlot);
        }
        if start_unix_millis % mode.slot_millis() != 0 {
            return Err(RadioValueError::MisalignedSlot);
        }
        Ok(Self {
            mode,
            start_unix_millis,
        })
    }

    /// Returns the synchronized mode defining this slot.
    #[must_use]
    pub const fn mode(self) -> OperatingMode {
        self.mode
    }

    /// Returns the UTC start as milliseconds since the Unix epoch.
    #[must_use]
    pub const fn start_unix_millis(self) -> i64 {
        self.start_unix_millis
    }
}

impl fmt::Display for UtcSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@{}", self.mode, self.start_unix_millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_display_and_wire_fixtures() {
        let dial = DialFrequency::from_hz(14_074_000).unwrap();
        let audio = AudioFrequency::from_hz(1_500).unwrap();
        let power = Power::from_milliwatts(5_000).unwrap();
        let slot = UtcSlot::new(OperatingMode::Ft8, 1_721_798_400_000).unwrap();
        assert_eq!(Band::B20m.to_string(), "20m");
        assert_eq!(dial.to_string(), "14074000");
        assert_eq!(serde_json::to_string(&dial).unwrap(), "14074000");
        assert_eq!(serde_json::to_string(&audio).unwrap(), "1500");
        assert_eq!(serde_json::to_string(&power).unwrap(), "5000");
        assert_eq!(
            serde_json::to_string(&slot).unwrap(),
            r#"{"mode":"FT8","start_unix_millis":1721798400000}"#
        );
    }

    #[test]
    fn invalid_radio_values_are_rejected() {
        assert_eq!("13m".parse::<Band>(), Err(RadioValueError::InvalidBand));
        assert!(DialFrequency::from_hz(99_999).is_err());
        assert!(AudioFrequency::from_hz(0).is_err());
        assert_eq!(
            Power::from_milliwatts(0),
            Err(RadioValueError::PowerOutOfRange)
        );
        assert_eq!(
            UtcSlot::new(OperatingMode::Ft8, 1),
            Err(RadioValueError::MisalignedSlot)
        );
        assert_eq!(
            UtcSlot::new(OperatingMode::Wspr, -120_000),
            Err(RadioValueError::NegativeSlot)
        );
    }
}
