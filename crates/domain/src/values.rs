use crate::DomainError;
use serde::{Deserialize, Serialize};
macro_rules! number {
    ($name:ident, $condition:expr, $message:literal) => {
        #[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "f64", into = "f64")]
        pub struct $name(f64);
        impl $name {
            pub fn new(value: f64) -> Result<Self, DomainError> {
                if value.is_finite() && ($condition)(value) {
                    Ok(Self(if value == 0.0 { 0.0 } else { value }))
                } else {
                    Err(DomainError::Number($message))
                }
            }

            pub fn get(self) -> f64 {
                self.0
            }
        }

        impl TryFrom<f64> for $name {
            type Error = DomainError;
            fn try_from(value: f64) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for f64 {
            fn from(value: $name) -> f64 {
                value.0
            }
        }
    };
}

number!(
    NonNegative,
    |v: f64| v >= 0.0,
    "expected finite nonnegative number"
);
number!(
    PositivePrice,
    |v: f64| v > 0.0,
    "expected finite positive price"
);
number!(
    UnitInterval,
    |v: f64| (0.0..=1.0).contains(&v),
    "expected value in [0,1]"
);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct TimestampMs(i64);
impl TimestampMs {
    pub fn new(value: i64) -> Result<Self, DomainError> {
        if value < 0 {
            Err(DomainError::Number("timestamp before epoch"))
        } else {
            Ok(Self(value))
        }
    }

    pub fn from_seconds(value: i64) -> Result<Self, DomainError> {
        Self::new(value.checked_mul(1000).ok_or(DomainError::TimeOverflow)?)
    }

    pub fn get(self) -> i64 {
        self.0
    }

    pub fn checked_add(self, millis: i64) -> Result<Self, DomainError> {
        Self::new(
            self.0
                .checked_add(millis)
                .ok_or(DomainError::TimeOverflow)?,
        )
    }

    pub fn closed_cutoff(self, resolution: Resolution, grace_ms: u64) -> Result<Self, DomainError> {
        let grace = i64::try_from(grace_ms).map_err(|_| DomainError::TimeOverflow)?;
        let time = self
            .0
            .checked_sub(grace)
            .filter(|x| *x >= 0)
            .ok_or(DomainError::TimeOverflow)?;
        Self::new(time / resolution.millis() * resolution.millis())
    }
}

impl TryFrom<i64> for TimestampMs {
    type Error = DomainError;
    fn try_from(v: i64) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}

impl From<TimestampMs> for i64 {
    fn from(v: TimestampMs) -> Self {
        v.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct Resolution(u64);
impl Resolution {
    pub fn new(seconds: u64) -> Result<Self, DomainError> {
        if seconds == 0 || seconds > 604800 {
            Err(DomainError::Number("resolution must be 1..604800 seconds"))
        } else {
            Ok(Self(seconds))
        }
    }

    pub fn seconds(self) -> u64 {
        self.0
    }

    pub fn millis(self) -> i64 {
        (self.0 * 1000) as i64
    }
}

impl TryFrom<u64> for Resolution {
    type Error = DomainError;
    fn try_from(v: u64) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}

impl From<Resolution> for u64 {
    fn from(v: Resolution) -> Self {
        v.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingReason {
    NotProvided,
    Unsupported,
    Stale,
    IncompleteCoverage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Knowledge<T> {
    Known { value: T },
    Unknown { reason: MissingReason },
}

impl<T> Knowledge<T> {
    pub fn known(value: T) -> Self {
        Self::Known { value }
    }

    pub fn unknown() -> Self {
        Self::Unknown {
            reason: MissingReason::NotProvided,
        }
    }

    pub fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Known { value } => Some(value),
            Self::Unknown { .. } => None,
        }
    }
}
