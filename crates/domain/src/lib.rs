#![forbid(unsafe_code)]
//! Validated identities and market facts; no runtime, network, or system clock.
pub mod identity;
pub mod market;
pub mod values;
pub use identity::*;
pub use market::*;
pub use values::*;
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DomainError {
    #[error("invalid identity: {0}")]
    Identity(String),
    #[error("invalid numeric value: {0}")]
    Number(&'static str),
    #[error("invalid market data: {0}")]
    Market(&'static str),
    #[error("time arithmetic overflow")]
    TimeOverflow,
}

mod amount;
pub use amount::AtomicAmount;
mod activity;
pub use activity::{ActivityId, ActivityLocation, ChainTransaction};
mod token_metrics;
pub use token_metrics::{MetricBounds, TokenMetric, TokenMetrics};

mod chain_state;
pub use chain_state::*;

#[cfg(test)]
mod tests;
