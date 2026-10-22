use domain::TimestampMs;
use state_api::StateError;
use std::time::{SystemTime, UNIX_EPOCH};
pub trait Clock: Send + Sync {
    fn now(&self) -> Result<TimestampMs, StateError>;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> Result<TimestampMs, StateError> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| StateError::Unavailable)?;
        let millis = i64::try_from(elapsed.as_millis()).map_err(|_| StateError::Unavailable)?;
        TimestampMs::new(millis).map_err(|_| StateError::Unavailable)
    }
}
