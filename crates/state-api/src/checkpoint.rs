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
        let mut tokens = std::collections::BTreeSet::new();
        let mut orders = std::collections::BTreeSet::new();
        if self.schema_version != 1
            || self.watch.len() > 1000
            || self.cursor.as_ref().is_some_and(|c| c.value.len() > 4096)
            || self.watch.iter().any(|entry| {
                entry.last_seen > self.saved_at
                    || entry.order > self.sequence
                    || !tokens.insert(&entry.candidate.token)
                    || !orders.insert(entry.order)
                    || entry.candidate.pool_hints.len() > 128
                    || entry
                        .candidate
                        .symbol
                        .as_ref()
                        .is_some_and(|s| s.len() > 256)
                    || entry
                        .candidate
                        .pool_hints
                        .iter()
                        .any(|p| p.chain() != entry.candidate.token.chain())
            })
            || serde_json::to_vec(self)
                .map_err(|_| StateError::InvalidRequest)?
                .len()
                > 4 * 1024 * 1024
        {
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
