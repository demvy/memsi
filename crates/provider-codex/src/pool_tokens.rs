//! Bounded identity cache; liquidity, age and other mutable facts are never cached here.
use domain::{PoolId, PoolSnapshot, TokenId};
use provider_api::ProviderError;
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Default)]
pub(crate) struct PoolTokens(Mutex<BTreeMap<PoolId, (TokenId, TokenId)>>);
impl PoolTokens {
    pub(crate) fn remember(&self, pool: &PoolSnapshot) -> Result<(), ProviderError> {
        pool.validate()?;
        let mut entries = self.0.lock().map_err(|_| ProviderError::WorkerPanicked)?;
        if let Some(tokens) = entries.get(&pool.pool) {
            if tokens != &(pool.token0.clone(), pool.token1.clone()) {
                return Err(ProviderError::InvalidData(
                    "pool token identity changed".into(),
                ));
            }

            return Ok(());
        }

        if entries.len() >= 512 {
            entries.pop_first();
        }

        entries.insert(
            pool.pool.clone(),
            (pool.token0.clone(), pool.token1.clone()),
        );
        Ok(())
    }

    pub(crate) fn side(
        &self,
        pool: &PoolId,
        token: &TokenId,
    ) -> Result<Option<&'static str>, ProviderError> {
        let entries = self.0.lock().map_err(|_| ProviderError::WorkerPanicked)?;
        let Some((token0, token1)) = entries.get(pool) else {
            return Ok(None);
        };

        if token == token0 {
            Ok(Some("token0"))
        } else if token == token1 {
            Ok(Some("token1"))
        } else {
            Err(ProviderError::InvalidRequest)
        }
    }
}
