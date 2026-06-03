use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, num::NonZeroU32, str::FromStr};
use thiserror::Error;

/// Canonical runtime/domain candle timeframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Timeframe {
    amount: NonZeroU32,
    unit: TimeframeUnit,
}

/// Supported timeframe units in runtime/domain code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeframeUnit {
    Minute,
    Hour,
    Day,
    Week,
}

/// Error returned when parsing or constructing an invalid timeframe.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TimeframeParseError {
    #[error("timeframe cannot be empty")]
    Empty,
    #[error("timeframe amount must be a positive integer")]
    InvalidAmount,
    #[error("unsupported timeframe unit '{0}'")]
    UnsupportedUnit(String),
}

impl Timeframe {
    pub fn new(amount: NonZeroU32, unit: TimeframeUnit) -> Self {
        Self { amount, unit }
    }

    pub fn minutes(amount: u32) -> Self {
        Self::from_positive_amount(amount, TimeframeUnit::Minute)
    }

    pub fn hours(amount: u32) -> Self {
        Self::from_positive_amount(amount, TimeframeUnit::Hour)
    }

    pub fn days(amount: u32) -> Self {
        Self::from_positive_amount(amount, TimeframeUnit::Day)
    }

    pub fn weeks(amount: u32) -> Self {
        Self::from_positive_amount(amount, TimeframeUnit::Week)
    }

    pub fn amount(self) -> NonZeroU32 {
        self.amount
    }

    pub fn unit(self) -> TimeframeUnit {
        self.unit
    }

    /// Duration of one candle for this timeframe in milliseconds.
    pub fn duration_ms(self) -> i64 {
        let multiplier = match self.unit {
            TimeframeUnit::Minute => 60_000_i64,
            TimeframeUnit::Hour => 3_600_000_i64,
            TimeframeUnit::Day => 86_400_000_i64,
            TimeframeUnit::Week => 7 * 86_400_000_i64,
        };

        i64::from(self.amount.get()) * multiplier
    }

    fn from_positive_amount(amount: u32, unit: TimeframeUnit) -> Self {
        Self {
            amount: NonZeroU32::new(amount).expect("timeframe amount must be positive"),
            unit,
        }
    }
}

impl fmt::Display for Timeframe {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unit = match self.unit {
            TimeframeUnit::Minute => "m",
            TimeframeUnit::Hour => "h",
            TimeframeUnit::Day => "d",
            TimeframeUnit::Week => "w",
        };

        write!(formatter, "{}{}", self.amount, unit)
    }
}

impl FromStr for Timeframe {
    type Err = TimeframeParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.is_empty() {
            return Err(TimeframeParseError::Empty);
        }

        let first_unit_index = input
            .char_indices()
            .find_map(|(index, character)| (!character.is_ascii_digit()).then_some(index))
            .ok_or(TimeframeParseError::UnsupportedUnit(String::new()))?;

        let amount_digits = &input[..first_unit_index];
        if first_unit_index == 0 || amount_digits.starts_with('0') {
            return Err(TimeframeParseError::InvalidAmount);
        }

        let amount = amount_digits
            .parse::<u32>()
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or(TimeframeParseError::InvalidAmount)?;

        let unit_suffix = &input[first_unit_index..];
        let unit = match unit_suffix {
            "m" => TimeframeUnit::Minute,
            "h" => TimeframeUnit::Hour,
            "d" => TimeframeUnit::Day,
            "w" => TimeframeUnit::Week,
            other => return Err(TimeframeParseError::UnsupportedUnit(other.to_owned())),
        };

        Ok(Self { amount, unit })
    }
}

impl Serialize for Timeframe {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Timeframe {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_canonical_timeframes() {
        assert_eq!("1m".parse::<Timeframe>(), Ok(Timeframe::minutes(1)));
        assert_eq!("15m".parse::<Timeframe>(), Ok(Timeframe::minutes(15)));
        assert_eq!("1h".parse::<Timeframe>(), Ok(Timeframe::hours(1)));
        assert_eq!("1d".parse::<Timeframe>(), Ok(Timeframe::days(1)));
        assert_eq!("1w".parse::<Timeframe>(), Ok(Timeframe::weeks(1)));
    }

    #[test]
    fn rejects_invalid_timeframe_strings() {
        for invalid in ["0m", "01m", "-1m", "15min", "", "1q", "1wk", "m", "1"] {
            assert!(
                invalid.parse::<Timeframe>().is_err(),
                "expected {invalid:?} to be invalid"
            );
        }
    }

    #[test]
    fn calculates_duration_ms() {
        assert_eq!(Timeframe::minutes(15).duration_ms(), 15 * 60_000);
        assert_eq!(Timeframe::hours(1).duration_ms(), 3_600_000);
        assert_eq!(Timeframe::days(1).duration_ms(), 86_400_000);
        assert_eq!(Timeframe::weeks(1).duration_ms(), 7 * 86_400_000);
    }

    #[test]
    fn display_round_trips_through_from_str() {
        for timeframe in [
            Timeframe::minutes(1),
            Timeframe::minutes(15),
            Timeframe::hours(1),
            Timeframe::days(1),
            Timeframe::weeks(1),
        ] {
            assert_eq!(timeframe.to_string().parse::<Timeframe>(), Ok(timeframe));
        }
    }

    #[test]
    fn serde_round_trips_as_canonical_string() {
        let serialized = serde_json::to_string(&Timeframe::hours(4)).unwrap();
        assert_eq!(serialized, "\"4h\"");
        assert_eq!(
            serde_json::from_str::<Timeframe>(&serialized).unwrap(),
            Timeframe::hours(4)
        );
    }
}
