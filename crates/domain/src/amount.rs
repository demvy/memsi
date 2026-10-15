//! Exact unsigned base-unit amounts; decimal strings keep JSON consumers lossless.
use crate::DomainError;
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, fmt};

const MAX_U256: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935";
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AtomicAmount(String);
impl AtomicAmount {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_U256.len()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
            || (value.len() == MAX_U256.len() && value.as_str() > MAX_U256)
        {
            return Err(DomainError::Number(
                "expected canonical uint256 decimal string",
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn is_zero(&self) -> bool {
        self.0 == "0"
    }
    pub fn checked_add(&self, other: &Self) -> Result<Self, DomainError> {
        let mut left = self.0.bytes().rev();
        let mut right = other.0.bytes().rev();
        let mut digits = Vec::with_capacity(MAX_U256.len() + 1);
        let mut carry = 0;
        loop {
            let a = left.next();
            let b = right.next();
            if a.is_none() && b.is_none() && carry == 0 {
                break;
            }
            let sum = a.map_or(0, |d| d - b'0') + b.map_or(0, |d| d - b'0') + carry;
            digits.push(b'0' + sum % 10);
            carry = sum / 10;
        }
        digits.reverse();
        Self::new(String::from_utf8(digits).expect("decimal digits"))
    }
    /// Exact display conversion, never floating-point arithmetic.
    pub fn units(&self, decimals: u8) -> String {
        let places = usize::from(decimals);
        if places == 0 {
            return self.0.clone();
        }
        if places >= self.0.len() {
            format!("0.{}{}", "0".repeat(places - self.0.len()), self.0)
        } else {
            let split = self.0.len() - places;
            format!("{}.{}", &self.0[..split], &self.0[split..])
        }
    }
}
impl Ord for AtomicAmount {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.cmp(&other.0))
    }
}
impl PartialOrd for AtomicAmount {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl From<u64> for AtomicAmount {
    fn from(value: u64) -> Self {
        Self(value.to_string())
    }
}
impl From<u128> for AtomicAmount {
    fn from(value: u128) -> Self {
        Self(value.to_string())
    }
}
impl TryFrom<String> for AtomicAmount {
    type Error = DomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<AtomicAmount> for String {
    fn from(value: AtomicAmount) -> Self {
        value.0
    }
}
impl fmt::Display for AtomicAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
