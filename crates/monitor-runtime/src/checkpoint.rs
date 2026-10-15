use super::*;
use state_api::MonitorCheckpoint;
impl Monitor<'_> {
    pub(super) async fn restore(&mut self, now: TimestampMs) -> Result<(), StateError> {
        if self.restored {
            return Ok(());
        }
        if let Some(mut checkpoint) = self
            .dependencies
            .state
            .load_checkpoint(&self.checkpoint_scope)
            .await?
        {
            checkpoint.validate()?;
            if checkpoint.cursor.as_ref().is_some_and(|cursor| {
                self.legacy_query_digest.as_ref() == Some(&cursor.query_digest)
            }) {
                checkpoint.cursor = None;
                checkpoint.next_discovery = now;
            }
            if checkpoint.saved_at > now
                || checkpoint.watch.len()
                    > self.dependencies.config.monitor.queue_capacity.min(1000)
                || checkpoint
                    .watch
                    .iter()
                    .any(|entry| entry.candidate.token.chain() != &self.query.chain)
                || checkpoint
                    .cursor
                    .as_ref()
                    .is_some_and(|cursor| cursor.query_digest != self.query.digest())
            {
                return Err(StateError::InvalidRequest);
            }
            self.cursor = checkpoint.cursor;
            self.next_discovery = checkpoint.next_discovery;
            self.sequence = checkpoint.sequence;
            self.watch = checkpoint
                .watch
                .into_iter()
                .map(|entry| (entry.candidate.token.clone(), entry))
                .collect();
        }
        self.restored = true;
        Ok(())
    }
    pub(super) async fn checkpoint(&self) -> Result<(), StateError> {
        self.dependencies
            .state
            .save_checkpoint(MonitorCheckpoint {
                schema_version: 1,
                scope: self.checkpoint_scope.clone(),
                saved_at: self.dependencies.clock.now()?,
                cursor: self.cursor.clone(),
                next_discovery: self.next_discovery,
                sequence: self.sequence,
                watch: self.watch.values().cloned().collect(),
            })
            .await
    }
}
