use super::*;
use async_trait::async_trait;
use signal_api::SignalDraft;
use std::collections::BTreeSet;
#[async_trait]
impl SignalRepository for MemoryState {
    fn durability(&self) -> Durability {
        Durability::Volatile
    }

    async fn accept_signal_and_deliveries(
        &self,
        draft: SignalDraft,
        targets: &[DeliveryTarget],
        now: TimestampMs,
    ) -> Result<Acceptance, StateError> {
        state_api::validate_acceptance(&draft, targets, now)?;
        let cooldown_end = now
            .checked_add(draft.cooldown_secs as i64 * 1000)
            .map_err(|_| StateError::InvalidRequest)?;
        let mut inner = self.lock()?;
        self.maintain(&mut inner, now);
        if inner.signals.contains_key(&draft.signal.id)
            || inner.dedupe.contains_key(&draft.dedupe_key)
        {
            return Ok(Acceptance::Suppressed);
        }

        let staged = targets
            .iter()
            .map(|target| {
                let id = ContentHash::of(&(
                    1u8,
                    &draft.signal.id,
                    &target.sink,
                    &target.destination,
                    0u32,
                ))
                .map_err(|_| StateError::InvalidRequest)?;
                let notification = Notification {
                    delivery_id: id.clone(),
                    signal: draft.signal.clone(),
                    links: vec![],
                    destination: target.destination.clone(),
                    part_index: 0,
                };

                Ok((
                    id,
                    DeliveryRow {
                        sink: target.sink.clone(),
                        notification,
                        status: DeliveryStatus::Pending,
                        due: Some(now),
                        attempt: 0,
                        lease: None,
                        receipt: None,
                    },
                ))
            })
            .collect::<Result<Vec<_>, StateError>>()?;
        let active_signals: BTreeSet<_> = inner
            .deliveries
            .values()
            .filter(|row| !row.terminal())
            .map(|row| row.notification.signal.id.clone())
            .collect();
        let removable_signals: Vec<_> = inner
            .signals
            .values()
            .filter(|signal| !active_signals.contains(&signal.id))
            .map(|signal| (signal.created_at, signal.id.clone()))
            .collect();
        let removable_deliveries = inner
            .deliveries
            .values()
            .filter(|row| row.terminal())
            .count();
        let signals_needed = (inner.signals.len() + 1).saturating_sub(self.limits.signals);
        let deliveries_needed =
            (inner.deliveries.len() + staged.len()).saturating_sub(self.limits.deliveries);
        if removable_signals.len() < signals_needed
            || removable_deliveries < deliveries_needed
            || inner.dedupe.len() >= self.limits.dedupe_keys
        {
            return Err(StateError::Capacity);
        }

        let mut removable_signals = removable_signals;
        removable_signals.sort();
        for (_, id) in removable_signals.into_iter().take(signals_needed) {
            inner.signals.remove(&id);
        }

        let mut terminal: Vec<_> = inner
            .deliveries
            .iter()
            .filter(|(_, row)| row.terminal())
            .map(|(id, row)| (row.notification.signal.created_at, id.clone()))
            .collect();
        terminal.sort();
        for (_, id) in terminal.into_iter().take(deliveries_needed) {
            inner.deliveries.remove(&id);
        }

        let id = draft.signal.id.clone();
        inner.dedupe.insert(
            draft.dedupe_key,
            DedupeEntry {
                signal: id.clone(),
                until: cooldown_end,
            },
        );
        inner.signals.insert(id.clone(), draft.signal);
        inner.deliveries.extend(staged);
        Ok(Acceptance::Accepted(id))
    }

    async fn signal_history(
        &self,
        before: Option<&SignalCursor>,
        limit: usize,
    ) -> Result<Vec<Signal>, StateError> {
        if !(1..=200).contains(&limit) {
            return Err(StateError::InvalidRequest);
        }

        let inner = self.lock()?;
        let mut signals: Vec<_> = inner
            .signals
            .values()
            .filter(|signal| {
                before.is_none_or(|cursor| {
                    signal.created_at < cursor.created_at
                        || (signal.created_at == cursor.created_at && signal.id > cursor.id)
                })
            })
            .collect();
        signals.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(signals.into_iter().take(limit).cloned().collect())
    }

    async fn list_signals(&self, limit: usize) -> Result<Vec<Signal>, StateError> {
        if limit == 0 || limit > 1000 {
            return Err(StateError::InvalidRequest);
        }

        let inner = self.lock()?;
        let mut signals: Vec<_> = inner.signals.values().cloned().collect();
        signals.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        signals.truncate(limit);
        Ok(signals)
    }
}
