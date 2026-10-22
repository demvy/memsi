use std::{error::Error, fmt};
/// Errors caused by invalid configuration or input values.
#[derive(Clone, Debug, PartialEq)]
pub enum RangeError {
    EmptyInput,
    NonFinitePrice { index: usize, value: f64 },
    NonPositivePrice { index: usize, value: f64 },
    InvalidConfig(&'static str),
}

impl fmt::Display for RangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => write!(formatter, "input is empty"),
            Self::NonFinitePrice { index, value } => {
                write!(formatter, "price at index {index} is not finite: {value}")
            }

            Self::NonPositivePrice { index, value } => {
                write!(formatter, "price at index {index} is not positive: {value}")
            }

            Self::InvalidConfig(message) => {
                write!(formatter, "invalid detector config: {message}")
            }
        }
    }
}

impl Error for RangeError {}
