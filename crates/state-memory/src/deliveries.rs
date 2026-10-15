use super::*;
use async_trait::async_trait;
#[async_trait]
impl DeliveryRepository for MemoryState {
    async fn claim_due(
        &self,
        worker: &InstanceId,
        now: TimestampMs,
        lease_secs: u64,
        limit: usize,
    ) -> Result<Vec<DeliveryLease>, StateError> {
        if lease_secs == 0 || lease_secs > 300 || limit == 0 || limit > 64 {
            return Err(StateError::InvalidRequest);
        }
        let lease_until = now
            .checked_add(lease_secs as i64 * 1000)
            .map_err(|_| StateError::InvalidRequest)?;
        let mut inner = self.lock()?;
        self.maintain(&mut inner, now);
        let mut due: Vec<_> = inner
            .deliveries
            .iter()
            .filter(|(_, row)| {
                !row.terminal()
                    && !matches!(row.status, DeliveryStatus::Sending)
                    && row.due.is_some_and(|due| due <= now)
            })
            .map(|(id, row)| (row.due.unwrap(), id.clone()))
            .collect();
        due.sort();
        let mut leases = Vec::new();
        for (_, id) in due.into_iter().take(limit) {
            let row = inner.deliveries.get_mut(&id).unwrap();
            row.attempt = row.attempt.checked_add(1).ok_or(StateError::Capacity)?;
            let token = ContentHash::of(&(1u8, &id, worker, row.attempt, now))
                .map_err(|_| StateError::InvalidRequest)?;
            let lease = DeliveryLease {
                delivery_id: id,
                lease_token: token,
                lease_until,
                sink: row.sink.clone(),
                notification: row.notification.clone(),
                attempt: row.attempt,
            };
            row.status = DeliveryStatus::Sending;
            row.lease = Some(lease.clone());
            row.due = None;
            leases.push(lease);
        }
        Ok(leases)
    }
    async fn complete(
        &self,
        lease: &DeliveryLease,
        receipt: DeliveryReceipt,
        now: TimestampMs,
    ) -> Result<(), StateError> {
        if receipt.accepted_at > now
            || receipt
                .remote_message_id
                .as_ref()
                .is_some_and(|id| id.len() > 256)
        {
            return Err(StateError::InvalidRequest);
        }
        let mut inner = self.lock()?;
        let row = owned(&mut inner, lease, now)?;
        row.status = DeliveryStatus::Delivered;
        row.receipt = Some(receipt);
        row.lease = None;
        row.due = None;
        Ok(())
    }
    async fn retry(
        &self,
        lease: &DeliveryLease,
        due: TimestampMs,
        unknown_outcome: bool,
        now: TimestampMs,
    ) -> Result<(), StateError> {
        if due < now {
            return Err(StateError::InvalidRequest);
        }
        let mut inner = self.lock()?;
        let row = owned(&mut inner, lease, now)?;
        row.status = if unknown_outcome {
            DeliveryStatus::UnknownOutcome
        } else if due >= row.notification.signal.expires_at {
            DeliveryStatus::Expired
        } else {
            DeliveryStatus::RetryScheduled
        };
        row.due = (!row.terminal() && due < row.notification.signal.expires_at).then_some(due);
        row.lease = None;
        Ok(())
    }
    async fn fail(&self, lease: &DeliveryLease, now: TimestampMs) -> Result<(), StateError> {
        let mut inner = self.lock()?;
        let row = owned(&mut inner, lease, now)?;
        row.status = DeliveryStatus::FailedPermanent;
        row.lease = None;
        row.due = None;
        Ok(())
    }
}
fn owned<'a>(
    inner: &'a mut Inner,
    lease: &DeliveryLease,
    now: TimestampMs,
) -> Result<&'a mut DeliveryRow, StateError> {
    let row = inner
        .deliveries
        .get_mut(&lease.delivery_id)
        .ok_or(StateError::Conflict)?;
    if !matches!(row.status, DeliveryStatus::Sending)
        || !row.lease.as_ref().is_some_and(|current| {
            current.lease_token == lease.lease_token && current.lease_until > now
        })
    {
        return Err(StateError::Conflict);
    }
    Ok(row)
}
