//! Durable per-key admission accounting. Raw keys never enter the database or errors.
use app_config::ApiUsageConfig;
use domain::ContentHash;
use provider_api::{ApiKeyUsage, ProviderError};
use sqlx::{
    Connection, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::sync::{Mutex, MutexGuard};
pub(crate) struct UsageLedger {
    config: ApiUsageConfig,
    connection: Mutex<Option<SqliteConnection>>,
    initial: std::collections::BTreeMap<String, u64>,
    memory_initialized: AtomicBool,
}

impl UsageLedger {
    pub(crate) fn new(config: ApiUsageConfig) -> Self {
        Self::with_credentials(config, app_config::local_env)
    }

    fn with_credentials(config: ApiUsageConfig, resolve: impl Fn(&str) -> Option<String>) -> Self {
        let mut initial = std::collections::BTreeMap::new();
        for (name, used) in &config.initial_used {
            if let Some(key) = resolve(name)
                && let Ok(id) = Self::identity(&key)
            {
                initial
                    .entry(id)
                    .and_modify(|value: &mut u64| *value = (*value).max(*used))
                    .or_insert(*used);
            }
        }

        Self {
            initial,
            config,
            connection: Mutex::new(None),
            memory_initialized: AtomicBool::new(false),
        }
    }

    fn period(&self, now: SystemTime) -> Result<String, ProviderError> {
        let seconds = now
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ProviderError::UsageAccountingUnavailable)?
            .as_secs();
        let date = time::OffsetDateTime::from_unix_timestamp(
            i64::try_from(seconds).map_err(|_| ProviderError::UsageAccountingUnavailable)?,
        )
        .map_err(|_| ProviderError::UsageAccountingUnavailable)?;
        let mut year = date.year();
        let mut month = date.month() as u8;
        let boundary = self.config.reset_day.min(date.month().length(year));
        if date.day() < boundary {
            if month == 1 {
                year -= 1;
                month = 12;
            } else {
                month -= 1;
            }
        }

        let month_value =
            time::Month::try_from(month).map_err(|_| ProviderError::UsageAccountingUnavailable)?;
        let day = self.config.reset_day.min(month_value.length(year));
        Ok(format!("{year:04}-{month:02}-{day:02}"))
    }

    async fn connection(&self) -> Result<MutexGuard<'_, Option<SqliteConnection>>, ProviderError> {
        let mut slot = self.connection.lock().await;
        if slot.is_none() {
            if self.config.database == ":memory:" && self.memory_initialized.load(Ordering::Relaxed)
            {
                return Err(ProviderError::UsageAccountingUnavailable);
            }

            let options = SqliteConnectOptions::new()
                .filename(&self.config.database)
                .in_memory(self.config.database == ":memory:")
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Full)
                .busy_timeout(Duration::from_millis(250));
            let mut connection = SqliteConnection::connect_with(&options)
                .await
                .map_err(db_error)?;
            sqlx::query(
                    "CREATE TABLE IF NOT EXISTS api_usage (
                key_id TEXT NOT NULL, period TEXT NOT NULL, used INTEGER NOT NULL CHECK(used>=0),
                warning_claimed INTEGER NOT NULL DEFAULT 0, exhaustion_claimed INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(key_id,period))",
                )
                .execute(&mut connection)
                .await
                .map_err(db_error)?;
            self.memory_initialized.store(true, Ordering::Relaxed);
            *slot = Some(connection);
        }

        Ok(slot)
    }

    fn identity(key: &str) -> Result<String, ProviderError> {
        ContentHash::of(&("codex.api-key.v1", key))
            .map(|hash| hash.as_str().to_owned())
            .map_err(|_| ProviderError::UsageAccountingUnavailable)
    }

    fn initial_used(&self, id: &str, period: &str) -> u64 {
        if self.config.initial_period.as_deref() != Some(period) {
            return 0;
        }

        self.initial.get(id).copied().unwrap_or(0)
    }

    pub(crate) async fn charge_first(
        &self,
        keys: impl IntoIterator<Item = String>,
    ) -> Result<String, ProviderError> {
        for key in keys {
            if self.charge(&key).await? {
                return Ok(key);
            }
        }

        Err(ProviderError::QuotaExhausted)
    }

    pub(crate) async fn charge(&self, key: &str) -> Result<bool, ProviderError> {
        self.charge_at(key, SystemTime::now()).await
    }

    async fn charge_at(&self, key: &str, now: SystemTime) -> Result<bool, ProviderError> {
        let id = Self::identity(key)?;
        let period = self.period(now)?;
        let initial_used = self.initial_used(&id, &period) as i64;
        let mut slot = self.connection().await?;
        let mut connection = slot.take().expect("initialized above");
        let result = async {
            let mut tx = connection
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(db_error)?;
            sqlx::query("INSERT OR IGNORE INTO api_usage(key_id,period,used) VALUES(?1,?2,?3)")
                .bind(&id)
                .bind(&period)
                .bind(initial_used)
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            let admitted = sqlx::query(
                "UPDATE api_usage SET used=used+1 WHERE key_id=?1 AND period=?2 AND used<?3",
            )
            .bind(&id)
            .bind(&period)
            .bind(self.config.monthly_limit as i64)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?
            .rows_affected()
                == 1;
            tx.commit().await.map_err(db_error)?;
            Ok(admitted)
        }
        .await;
        if result.is_ok() {
            self.memory_initialized.store(true, Ordering::Relaxed);
            *slot = Some(connection);
        } else {
            let _ = connection.close().await;
        }

        result
    }

    pub(crate) async fn status(&self, key: &str) -> Result<ApiKeyUsage, ProviderError> {
        self.status_at(key, SystemTime::now()).await
    }

    async fn status_at(&self, key: &str, now: SystemTime) -> Result<ApiKeyUsage, ProviderError> {
        let id = Self::identity(key)?;
        let period = self.period(now)?;
        let initial_used = self.initial_used(&id, &period) as i64;
        let mut slot = self.connection().await?;
        let connection = slot.as_mut().expect("initialized above");
        sqlx::query("INSERT OR IGNORE INTO api_usage(key_id,period,used) VALUES(?1,?2,?3)")
            .bind(&id)
            .bind(&period)
            .bind(initial_used)
            .execute(&mut *connection)
            .await
            .map_err(db_error)?;
        let used: i64 =
            sqlx::query_scalar("SELECT used FROM api_usage WHERE key_id=?1 AND period=?2")
                .bind(&id)
                .bind(&period)
                .fetch_one(&mut *connection)
                .await
                .map_err(db_error)?;
        Ok(ApiKeyUsage {
            key_id: id,
            period,
            used: u64::try_from(used).map_err(|_| ProviderError::UsageAccountingUnavailable)?,
            limit: self.config.monthly_limit,
            warn_at: self.config.warn_at,
        })
    }

    pub(crate) async fn claim_notice(
        &self,
        usage: &ApiKeyUsage,
        exhausted: bool,
    ) -> Result<bool, ProviderError> {
        let (query, threshold) = if exhausted {
            (
                "UPDATE api_usage SET exhaustion_claimed=1 WHERE key_id=?1 AND period=?2 AND exhaustion_claimed=0 AND used>=?3",
                usage.limit,
            )
        } else {
            (
                "UPDATE api_usage SET warning_claimed=1 WHERE key_id=?1 AND period=?2 AND warning_claimed=0 AND used>=?3",
                usage.warn_at,
            )
        };

        let mut slot = self.connection().await?;
        let connection = slot.as_mut().expect("initialized above");
        Ok(sqlx::query(query)
            .bind(&usage.key_id)
            .bind(&usage.period)
            .bind(threshold as i64)
            .execute(&mut *connection)
            .await
            .map_err(db_error)?
            .rows_affected()
            == 1)
    }
}

fn db_error(_: sqlx::Error) -> ProviderError {
    ProviderError::UsageAccountingUnavailable
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn config() -> ApiUsageConfig {
        ApiUsageConfig {
            monthly_limit: 3,
            warn_at: 2,
            database: ":memory:".into(),
            ..Default::default()
        }
    }

    fn date(year: i32, month: u8, day: u8) -> SystemTime {
        let timestamp =
            time::Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), day)
                .unwrap()
                .midnight()
                .assume_utc()
                .unix_timestamp();
        UNIX_EPOCH + Duration::from_secs(timestamp as u64)
    }

    #[test]
    fn billing_periods_clamp_month_end_and_roll_back_before_boundary() {
        let ledger = UsageLedger::new(ApiUsageConfig {
            reset_day: 31,
            ..config()
        });
        for (now, expected) in [
            (date(2024, 3, 1), "2024-02-29"),
            (date(2025, 3, 1), "2025-02-28"),
            (date(2025, 1, 1), "2024-12-31"),
            (date(2025, 4, 30), "2025-04-30"),
            (date(2025, 3, 31), "2025-03-31"),
        ] {
            assert_eq!(ledger.period(now).unwrap(), expected);
        }

        assert!(ledger.period(UNIX_EPOCH - Duration::from_secs(1)).is_err());
        assert_ne!(
            UsageLedger::identity("first").unwrap(),
            UsageLedger::identity("second").unwrap()
        );
        assert_eq!(UsageLedger::identity("first").unwrap().len(), 64);
    }

    #[tokio::test]
    async fn ledger_charges_exactly_to_limit_and_claims_each_notice_once() {
        let ledger = UsageLedger::new(config());
        assert_eq!(ledger.status("key").await.unwrap().used, 0);
        assert!(ledger.charge("key").await.unwrap());
        let early = ledger.status("key").await.unwrap();
        assert!(!ledger.claim_notice(&early, false).await.unwrap());
        assert!(ledger.charge("key").await.unwrap());
        let warning = ledger.status("key").await.unwrap();
        assert!(ledger.claim_notice(&warning, false).await.unwrap());
        assert!(!ledger.claim_notice(&warning, false).await.unwrap());
        assert!(!ledger.claim_notice(&warning, true).await.unwrap());
        assert!(ledger.charge("key").await.unwrap());
        assert!(!ledger.charge("key").await.unwrap());
        let exhausted = ledger.status("key").await.unwrap();
        assert_eq!(exhausted.used, 3);
        assert!(ledger.claim_notice(&exhausted, true).await.unwrap());
        assert!(!ledger.claim_notice(&exhausted, true).await.unwrap());
        assert_eq!(
            ledger
                .charge_first(vec!["key".into(), "fallback".into()])
                .await
                .unwrap(),
            "fallback"
        );
        assert!(matches!(
            ledger.charge_first(vec!["key".into()]).await,
            Err(ProviderError::QuotaExhausted)
        ));
    }

    #[tokio::test]
    async fn initial_usage_is_resolved_per_identity_and_applies_only_to_configured_period() {
        let now = date(2025, 1, 1);
        let ledger = UsageLedger::with_credentials(
            ApiUsageConfig {
                initial_period: Some("2025-01-01".into()),
                initial_used: BTreeMap::from([
                    ("FIRST".into(), 1),
                    ("ALIAS".into(), 2),
                    ("MISSING".into(), 3),
                ]),
                ..config()
            },
            |name| (name != "MISSING").then(|| "shared-key".into()),
        );
        assert_eq!(ledger.status_at("shared-key", now).await.unwrap().used, 2);
        assert!(ledger.charge_at("shared-key", now).await.unwrap());
        assert!(!ledger.charge_at("shared-key", now).await.unwrap());
        assert_eq!(
            ledger
                .status_at("shared-key", date(2025, 2, 1))
                .await
                .unwrap()
                .used,
            0
        );
        assert!(
            ledger
                .charge_at("shared-key", date(2025, 2, 1))
                .await
                .unwrap()
        );
        assert_eq!(ledger.status_at("other-key", now).await.unwrap().used, 0);
    }

    #[tokio::test]
    async fn database_failure_is_sanitized_and_does_not_admit_requests() {
        let ledger = UsageLedger::new(ApiUsageConfig {
            database: "/nonexistent/memsi/usage.sqlite".into(),
            ..config()
        });
        assert_eq!(
            ledger.charge("secret-key").await,
            Err(ProviderError::UsageAccountingUnavailable)
        );
    }
}
