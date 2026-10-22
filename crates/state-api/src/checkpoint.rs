use super::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchRecord {
    pub candidate: Candidate,
    pub next_due: TimestampMs,
    pub last_seen: TimestampMs,
    pub order: u64,
}

/// One monitor owns a state store. Scope changes invalidate scheduling, not signal history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MonitorCheckpoint {
    pub schema_version: u16,
    pub scope: ContentHash,
    pub saved_at: TimestampMs,
    pub cursor: Option<discovery_api::DiscoveryCursor>,
    pub next_discovery: TimestampMs,
    pub sequence: u64,
    pub watch: Vec<WatchRecord>,
}

impl MonitorCheckpoint {
    pub fn validate(&self) -> Result<(), StateError> {
        if self.schema_version != 1 {
            tracing::warn!("invalid monitor checkpoint: unsupported schema version (expected 1)");
            return Err(StateError::InvalidRequest);
        }

        if self.watch.len() > 1000 {
            tracing::warn!("invalid monitor checkpoint: watch list exceeds 1000 entries");
            return Err(StateError::InvalidRequest);
        }

        if self
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.value.len() > 4096)
        {
            tracing::warn!("invalid monitor checkpoint: cursor exceeds 4096 bytes");
            return Err(StateError::InvalidRequest);
        }

        let mut tokens = std::collections::BTreeSet::new();
        let mut orders = std::collections::BTreeSet::new();

        for (index, entry) in self.watch.iter().enumerate() {
            if entry.last_seen > self.saved_at {
                tracing::warn!(
                    "invalid monitor checkpoint: watch[{index}] last_seen is after saved_at"
                );
                return Err(StateError::InvalidRequest);
            }

            if entry.order > self.sequence {
                tracing::warn!(
                    "invalid monitor checkpoint: watch[{index}] order exceeds checkpoint sequence"
                );
                return Err(StateError::InvalidRequest);
            }

            if !tokens.insert(&entry.candidate.token) {
                tracing::warn!("invalid monitor checkpoint: watch[{index}] duplicate token");
                return Err(StateError::InvalidRequest);
            }

            if !orders.insert(entry.order) {
                tracing::warn!("invalid monitor checkpoint: watch[{index}] duplicate order");
                return Err(StateError::InvalidRequest);
            }

            if entry.candidate.pool_hints.len() > 128 {
                tracing::warn!(
                    "invalid monitor checkpoint: watch[{index}] pool hints exceed 128 entries"
                );
                return Err(StateError::InvalidRequest);
            }

            if entry
                .candidate
                .symbol
                .as_ref()
                .is_some_and(|symbol| symbol.len() > 256)
            {
                tracing::warn!(
                    "invalid monitor checkpoint: watch[{index}] symbol exceeds 256 bytes"
                );
                return Err(StateError::InvalidRequest);
            }

            if entry
                .candidate
                .pool_hints
                .iter()
                .any(|pool| pool.chain() != entry.candidate.token.chain())
            {
                tracing::warn!(
                    "invalid monitor checkpoint: watch[{index}] pool hint chain differs from token chain"
                );
                return Err(StateError::InvalidRequest);
            }
        }

        let payload = serde_json::to_vec(self).map_err(|_| {
            tracing::warn!("invalid monitor checkpoint: serialization failed");
            StateError::InvalidRequest
        })?;

        if payload.len() > 4 * 1024 * 1024 {
            tracing::warn!("invalid monitor checkpoint: serialized payload exceeds 4 MiB");
            return Err(StateError::InvalidRequest);
        }

        Ok(())
    }
}

#[async_trait]
pub trait MonitorRepository: Send + Sync {
    async fn load_checkpoint(
        &self,
        scope: &ContentHash,
    ) -> Result<Option<MonitorCheckpoint>, StateError>;
    async fn save_checkpoint(&self, checkpoint: MonitorCheckpoint) -> Result<(), StateError>;
}
