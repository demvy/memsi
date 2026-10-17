#![forbid(unsafe_code)]
use domain::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventEnvelope {
    pub id: ContentHash,
    pub occurred_at: TimestampMs,
    pub aggregate: String,
    pub sequence: u64,
    pub schema_version: u16,
    pub payload: LifecycleEvent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleEvent {
    CandidateObserved {
        token: TokenId,
    },
    AnalysisCompleted {
        snapshot: ContentHash,
    },
    StrategyMatched {
        instance: InstanceId,
        token: TokenId,
    },
    SignalAccepted {
        signal: ContentHash,
    },
    NotificationDelivered {
        delivery: ContentHash,
    },
    ProviderHealthChanged {
        provider: ProviderId,
        status: String,
    },
}

#[derive(Clone, Debug)]
pub struct PublishReport {
    pub accepted_subscribers: usize,
    pub dropped_subscribers: usize,
}

pub trait EventPublisher: Send + Sync {
    fn publish(&self, event: EventEnvelope) -> PublishReport;
}

impl EventEnvelope {
    pub fn new(
        occurred_at: TimestampMs,
        aggregate: String,
        sequence: u64,
        payload: LifecycleEvent,
    ) -> Result<Self, DomainError> {
        let mut event = Self {
            id: ContentHash::of_bytes(b""),
            occurred_at,
            aggregate,
            sequence,
            schema_version: 1,
            payload,
        };

        event.validate_shape()?;
        event.id = event.digest()?;
        Ok(event)
    }

    fn digest(&self) -> Result<ContentHash, DomainError> {
        ContentHash::of(&(
            "lifecycle.event.v1",
            self.occurred_at,
            &self.aggregate,
            self.sequence,
            self.schema_version,
            &self.payload,
        ))
        .map_err(|_| DomainError::Market("invalid lifecycle event"))
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        self.validate_shape()?;
        if self.id != self.digest()? {
            return Err(DomainError::Market("invalid lifecycle event identity"));
        }

        Ok(())
    }

    fn validate_shape(&self) -> Result<(), DomainError> {
        if self.schema_version != 1
            || self.sequence == 0
            || self.aggregate.is_empty()
            || self.aggregate.len() > 256
            || self.aggregate.chars().any(char::is_control)
        {
            return Err(DomainError::Market("invalid lifecycle event"));
        }

        if let LifecycleEvent::ProviderHealthChanged { status, .. } = &self.payload
            && (status.is_empty() || status.len() > 128 || status.chars().any(char::is_control))
        {
            return Err(DomainError::Market("invalid provider health event"));
        }

        Ok(())
    }
}

mod runtime;
pub use runtime::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> EventEnvelope {
        EventEnvelope::new(
            TimestampMs::new(100).unwrap(),
            "provider.test".into(),
            1,
            LifecycleEvent::ProviderHealthChanged {
                provider: ProviderId::new("test").unwrap(),
                status: "healthy".into(),
            },
        )
        .unwrap()
    }

    #[test]
    fn envelope_identity_is_stable_and_detects_tampering() {
        let valid = event();
        assert!(valid.validate().is_ok());
        assert_eq!(valid.id, event().id);
        for field in 0..6 {
            let mut invalid = valid.clone();
            match field {
                0 => invalid.id = ContentHash::of_bytes(b"tampered"),
                1 => invalid.occurred_at = TimestampMs::new(101).unwrap(),
                2 => invalid.aggregate = "other".into(),
                3 => invalid.sequence = 2,
                4 => invalid.schema_version = 2,
                _ => {
                    invalid.payload = LifecycleEvent::AnalysisCompleted {
                        snapshot: ContentHash::of_bytes(b"snapshot"),
                    }
                }
            }

            assert!(invalid.validate().is_err());
        }
    }

    #[test]
    fn envelopes_reject_invalid_shape_and_health_status() {
        for aggregate in ["", "bad\n", &"a".repeat(257)] {
            assert!(
                EventEnvelope::new(
                    TimestampMs::new(0).unwrap(),
                    aggregate.into(),
                    1,
                    event().payload
                )
                .is_err()
            );
        }

        assert!(
            EventEnvelope::new(
                TimestampMs::new(0).unwrap(),
                "test".into(),
                0,
                event().payload
            )
            .is_err()
        );
        for status in ["", "bad\n", &"a".repeat(129)] {
            assert!(
                EventEnvelope::new(
                    TimestampMs::new(0).unwrap(),
                    "test".into(),
                    1,
                    LifecycleEvent::ProviderHealthChanged {
                        provider: ProviderId::new("test").unwrap(),
                        status: status.into()
                    }
                )
                .is_err()
            );
        }
    }
}
