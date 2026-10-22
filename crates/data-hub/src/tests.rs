use super::*;
use test_support::market as fixtures;

struct Provider {
    calls: AtomicU64,
    outcome: Result<CandleSeries, ProviderError>,
    panic: bool,
}

#[async_trait::async_trait]
impl CandleProvider for Provider {
    async fn candles(
        &self,
        _: &CandleRequest,
        _: &RequestContext,
    ) -> Result<CandleSeries, ProviderError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        tokio::task::yield_now().await;
        assert!(!self.panic, "simulated provider panic");
        self.outcome.clone()
    }
}

fn request(series: &CandleSeries) -> CandleRequest {
    CandleRequest {
        pool: series.pool.clone(),
        priced_token: series.priced_token.clone(),
        price_unit: series.price_unit.clone(),
        resolution: series.resolution,
        from_inclusive: series.candles[0].open_time,
        end_exclusive: series.end_exclusive,
        min_bars: series.candles.len(),
        max_bars: series.candles.len(),
    }
}

fn context() -> RequestContext {
    RequestContext {
        priority: RequestPriority::default(),
        deadline: Instant::now() + Duration::from_secs(60),
        cancellation: Cancellation::default(),
    }
}

fn provider() -> Arc<Provider> {
    Arc::new(Provider {
        calls: AtomicU64::new(0),
        outcome: Ok(fixtures::series(&[1.0; 4], &[1.0; 4])),
        panic: false,
    })
}

#[test]
fn hub_constructor_checks_capacity_ttl_and_timeout_bounds() {
    for (capacity, ttl, timeout) in [
        (0, 1, 1),
        (1025, 1, 1),
        (1, 0, 1),
        (1, 3601, 1),
        (1, 1, 0),
        (1, 1, 301),
    ] {
        assert!(
            DataHub::new(
                provider(),
                capacity,
                Duration::from_secs(ttl),
                Duration::from_secs(timeout)
            )
            .is_err()
        );
    }
}

#[tokio::test]
async fn concurrent_requests_join_one_flight_and_success_is_cached() {
    let provider = provider();
    let series = provider.outcome.clone().unwrap();
    let hub = DataHub::new(
        provider.clone(),
        2,
        Duration::from_secs(60),
        Duration::from_secs(10),
    )
    .unwrap();
    let request = request(&series);
    let context = context();
    let (first, second) = tokio::join!(
        hub.candles(&request, &context),
        hub.candles(&request, &context)
    );
    assert!(Arc::ptr_eq(&first.unwrap(), &second.unwrap()));
    let cached = hub.candles(&request, &context).await.unwrap();
    assert!(Arc::ptr_eq(
        &cached,
        &hub.candles(&request, &context).await.unwrap()
    ));
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    let stats = hub.stats();
    assert_eq!(stats.upstream_attempts, 1);
    assert_eq!(stats.singleflight_joins, 1);
    assert_eq!(stats.cache_hits, 2);
}

#[tokio::test]
async fn provider_errors_and_panics_are_not_retained_as_successful_cache_entries() {
    for panic in [false, true] {
        let provider = Arc::new(Provider {
            calls: AtomicU64::new(0),
            outcome: Err(ProviderError::Transport),
            panic,
        });
        let hub = DataHub::new(
            provider.clone(),
            1,
            Duration::from_secs(60),
            Duration::from_secs(10),
        )
        .unwrap();
        let request = request(&fixtures::series(&[1.0; 4], &[1.0; 4]));
        for _ in 0..2 {
            let expected = if panic {
                ProviderError::WorkerPanicked
            } else {
                ProviderError::Transport
            };
            assert_eq!(
                hub.candles(&request, &context()).await.unwrap_err(),
                expected
            );
        }

        assert_eq!(provider.calls.load(Ordering::Relaxed), 2);
    }
}

#[tokio::test]
async fn cancelled_requests_never_start_upstream_work() {
    let provider = provider();
    let hub = DataHub::new(
        provider.clone(),
        1,
        Duration::from_secs(60),
        Duration::from_secs(10),
    )
    .unwrap();
    let context = context();
    context.cancellation.cancel();
    assert_eq!(
        hub.candles(&request(&provider.outcome.clone().unwrap()), &context)
            .await
            .unwrap_err(),
        ProviderError::Cancelled
    );
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn response_validation_rejects_mismatches_and_recomputes_the_input_hash() {
    let series = fixtures::series(&[1.0; 4], &[1.0; 4]);
    let request = request(&series);
    let result = validate_response(series.clone(), &request).unwrap();
    assert_eq!(
        result.provenance.input_hash,
        ContentHash::of(&(&request, &series.candles)).unwrap()
    );
    for field in 0..5 {
        let mut wrong = request.clone();
        match field {
            0 => wrong.pool = fixtures::pool_id(99),
            1 => wrong.priced_token = fixtures::token(99),
            2 => {
                wrong.price_unit = PriceUnit::QuotePerToken {
                    quote: fixtures::token(99),
                }
            }
            3 => wrong.max_bars = 3,
            _ => wrong.from_inclusive = fixtures::timestamp(60_000),
        }

        assert!(validate_response(series.clone(), &wrong).is_err());
    }
}

#[test]
fn incremental_requests_refresh_two_bars_and_merge_revisions_with_new_provenance() {
    let prefix = fixtures::series(&[1.0; 4], &[1.0; 4]);
    let full_series = fixtures::series(&[1.0; 6], &[1.0; 6]);
    let full = request(&full_series);
    assert!(incremental::can_extend(&prefix, &full));
    assert!(!incremental::can_extend(&prefix, &request(&prefix)));
    let delta = incremental::delta_request(&prefix, &full).unwrap();
    assert_eq!(delta.from_inclusive, fixtures::timestamp(120_000));
    assert_eq!(delta.min_bars, 4);
    assert_eq!(delta.max_bars, 4);
    let mut revised = full_series.clone();
    revised.candles = full_series.candles[2..].to_vec().into();
    revised.provenance.input_hash = ContentHash::of_bytes(b"new");
    let merged = incremental::merge(&prefix, revised.clone(), &delta, &full).unwrap();
    assert_eq!(merged.candles.len(), 6);
    assert_eq!(merged.provenance.input_hash, revised.provenance.input_hash);
    assert!(merged.validate(6).is_ok());
    for field in 0..3 {
        let mut incompatible = revised.clone();
        match field {
            0 => incompatible.volume_unit = VolumeUnit::Unknown,
            1 => incompatible.provenance.provider = ProviderId::new("other").unwrap(),
            _ => incompatible.provenance.contract_version = "other".into(),
        }

        assert!(incremental::merge(&prefix, incompatible, &delta, &full).is_err());
    }
}

#[test]
fn selection_ranks_by_liquidity_then_volume_and_identity_and_rejects_duplicates() {
    let mut small = fixtures::pool(10);
    small.liquidity_usd = Knowledge::known(fixtures::observation(100.0));
    let large = fixtures::pool(11);
    let policy = selection::SelectionPolicy::default();
    let plan = selection::rank(
        &fixtures::token(1),
        vec![small.clone(), large.clone()],
        &policy,
        fixtures::timestamp(600_000),
    )
    .unwrap();
    assert_eq!(
        plan.ranked
            .iter()
            .map(|pool| &pool.pool)
            .collect::<Vec<_>>(),
        [&large.pool, &small.pool]
    );
    assert!(plan.excluded.is_empty());
    assert!(
        selection::rank(
            &fixtures::token(1),
            vec![large.clone(), large],
            &policy,
            fixtures::timestamp(600_000)
        )
        .is_err()
    );
    for max_probes in [0, 21] {
        assert!(
            selection::rank(
                &fixtures::token(1),
                vec![],
                &selection::SelectionPolicy {
                    max_probes,
                    ..Default::default()
                },
                fixtures::timestamp(600_000)
            )
            .is_err()
        );
    }
}

#[test]
fn selection_explains_each_exclusion_without_silently_dropping_pools() {
    for (policy, token, expected) in [
        (
            selection::SelectionPolicy::default(),
            fixtures::token(99),
            "token_not_in_pool",
        ),
        (
            selection::SelectionPolicy {
                purpose: selection::Purpose::ConcentratedLp,
                ..Default::default()
            },
            fixtures::token(1),
            "concentrated_pool_not_verified",
        ),
        (
            selection::SelectionPolicy {
                quote_tokens: vec![fixtures::token(99)],
                ..Default::default()
            },
            fixtures::token(1),
            "quote_not_allowed",
        ),
        (
            selection::SelectionPolicy {
                min_liquidity: Some(NonNegative::new(20_000.0).unwrap()),
                ..Default::default()
            },
            fixtures::token(1),
            "liquidity_missing_stale_or_below_minimum",
        ),
        (
            selection::SelectionPolicy {
                min_age_secs: 601,
                ..Default::default()
            },
            fixtures::token(1),
            "pool_age_missing_or_too_young",
        ),
    ] {
        let plan = selection::rank(
            &token,
            vec![fixtures::pool(10)],
            &policy,
            fixtures::timestamp(600_000),
        )
        .unwrap();
        assert!(plan.ranked.is_empty());
        assert_eq!(plan.excluded.len(), 1);
        assert_eq!(plan.excluded[0].reason, expected);
    }
}
