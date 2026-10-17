use super::*;
use analysis_model::*;
#[path = "../../../test-support/decision.rs"]
mod fixtures;
use fixtures::market;

fn config() -> StrategyConfig {
    StrategyConfig {
        filters: Default::default(),
        signal: None,
        metrics_max_age_secs: 60,
        thresholds: Some(ThresholdOverrides {
            liquidity_usd: Some(0.0),
            volume_24h_usd: Some(0.0),
            age_secs: Some(0),
            ..Default::default()
        }),
        research: None,
        range_analyzer: InstanceId::new("range").unwrap(),
        volume_analyzer: InstanceId::new("volume").unwrap(),
        trend: None,
        research_only: true,
        quote_tokens: vec![],
        protocols: vec![],
    }
}

fn build_strategy(config: StrategyConfig) -> RangeStrategy {
    RangeStrategy::new(
        InstanceId::new("test").unwrap(),
        StrategyMode::Active,
        Profile::High,
        config,
    )
    .unwrap()
}

struct Inputs {
    candidate: Candidate,
    pool: PoolSnapshot,
    range: Option<RangeAnalysis>,
    volume: Option<VolumeAnalysis>,
    trend: Option<TrendAnalysis>,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            candidate: market::candidate(),
            pool: market::pool(10),
            range: Some(fixtures::range()),
            volume: Some(fixtures::volume()),
            trend: None,
        }
    }
}

impl Inputs {
    fn evaluate(&self, strategy: &RangeStrategy) -> Result<StrategyDecision, StrategyError> {
        strategy.evaluate(&StrategyContext {
            candles: None,
            candidate: &self.candidate,
            pool: &self.pool,
            range: self.range.as_ref(),
            volume: self.volume.as_ref(),
            trend: self.trend.as_ref(),
            snapshot_hash: ContentHash::of_bytes(b"snapshot"),
            input_snapshot_hash: ContentHash::of_bytes(b"input"),
            as_of: market::timestamp(600_000),
        })
    }
}

#[test]
fn profiles_threshold_overrides_and_constructor_preserve_requirements_and_identity() {
    for (profile, recheck) in [
        (Profile::Low, 1800),
        (Profile::Medium, 1200),
        (Profile::High, 900),
    ] {
        assert!(profile.thresholds().validate().is_ok());
        let strategy = RangeStrategy::new(
            InstanceId::new("test").unwrap(),
            StrategyMode::Shadow,
            profile,
            config(),
        )
        .unwrap();
        assert_eq!(strategy.descriptor().id.as_str(), profile.plugin_id());
        assert_eq!(strategy.instance().as_str(), "test");
        assert_eq!(strategy.mode(), StrategyMode::Shadow);
        assert_eq!(strategy.token_filter_recheck_secs(), recheck);
        assert_eq!(
            strategy.config_digest(),
            RangeStrategy::new(
                InstanceId::new("other").unwrap(),
                StrategyMode::Active,
                profile,
                config()
            )
            .unwrap()
            .config_digest()
        );
        let requirements = strategy.requirements();
        assert_eq!(requirements.analyses.len(), 2);
        assert_eq!(requirements.candles.min_bars, 270);
        assert_eq!(requirements.candles.desired_bars, 300);
        assert!(!requirements.pool.concentrated_only);
        assert_eq!(requirements.pool.min_pool_age_secs, 0);
        assert!(!strategy.uses_token_metrics());
        assert!(strategy.discovery_token_filters().is_empty());
        assert_eq!(
            strategy.required_token_metrics(),
            Some(vec![TokenMetric::ProviderSeenAgeSecs])
        );
        assert!(
            strategy
                .next_market_check(market::timestamp(600_000), 5000)
                .is_none()
        );
    }
}

#[test]
fn configuration_rejects_invalid_thresholds_filters_and_missing_live_inputs() {
    for field in 0..7 {
        let mut invalid = config();
        match field {
            0 => invalid.metrics_max_age_secs = 0,
            1 => invalid.metrics_max_age_secs = 86401,
            2 => invalid.thresholds.as_mut().unwrap().confidence = Some(f64::NAN),
            3 => invalid.thresholds.as_mut().unwrap().max_width_pct = Some(1.0),
            4 => invalid
                .filters
                .insert(
                    TokenMetric::DevSupplyPct,
                    MetricBounds {
                        gt: Some(101.0),
                        ..Default::default()
                    },
                )
                .map(|_| ())
                .unwrap_or(()),
            5 => invalid.research_only = false,
            _ => {
                invalid.research = Some(ResearchPolicy {
                    min_position_pct: 80.0,
                    max_position_pct: 20.0,
                    ..Default::default()
                })
            }
        }

        assert!(
            RangeStrategy::new(
                InstanceId::new("test").unwrap(),
                StrategyMode::Active,
                Profile::High,
                invalid
            )
            .is_err()
        );
    }
}

#[test]
fn strategy_evaluation_matches_valid_inputs_and_preserves_evidence() {
    let strategy = build_strategy(config());
    let inputs = Inputs::default();
    let decision = inputs.evaluate(&strategy).unwrap();
    assert_eq!(decision.verdict, Verdict::Match);
    assert_eq!(decision.watch.after_secs, 120);
    assert_eq!(
        decision.evidence.snapshot_hash,
        ContentHash::of_bytes(b"snapshot")
    );
    assert!(
        decision
            .warnings
            .iter()
            .any(|warning| warning == "ResearchOnly")
    );
    assert_eq!(decision.score.unwrap().get(), 0.95);
    assert!(matches!(
        strategy.prefilter(&inputs.candidate, market::timestamp(600_000)),
        PrefilterDecision::Continue
    ));
}

#[test]
fn missing_stale_and_incompatible_inputs_produce_unavailable_decisions() {
    let strategy = build_strategy(config());
    for field in 0..9 {
        let mut inputs = Inputs::default();
        match field {
            0 => inputs.range = None,
            1 => inputs.volume = None,
            2 => inputs.pool.liquidity_usd = Knowledge::unknown(),
            3 => inputs.pool.volume_24h_usd = Knowledge::unknown(),
            4 => inputs.pool.created_at = Knowledge::unknown(),
            5 => inputs.volume.as_mut().unwrap().relative_volume = None,
            6 => inputs.volume.as_mut().unwrap().unit = VolumeUnit::Unknown,
            7 => inputs.range.as_mut().unwrap().state = RangeState::InsufficientData,
            _ => inputs.range.as_mut().unwrap().range = None,
        }

        let decision = inputs.evaluate(&strategy).unwrap();
        assert_eq!(decision.verdict, Verdict::Unavailable, "field={field}");
        assert!(!decision.reasons.is_empty());
    }
}

#[test]
fn range_and_volume_rules_distinguish_reject_from_watch() {
    let strategy = build_strategy(config());
    for (field, verdict, code) in [
        (0, Verdict::Reject, "NO_RANGE"),
        (1, Verdict::Reject, "RANGE_WIDTH_OUTSIDE"),
        (2, Verdict::Watch, "RANGE_UNCONFIRMED"),
        (3, Verdict::Watch, "RANGE_CONFIDENCE_LOW"),
        (4, Verdict::Watch, "PRICE_OUTSIDE_RANGE"),
        (5, Verdict::Reject, "VOLUME_INACTIVE"),
        (6, Verdict::Reject, "RELATIVE_VOLUME_LOW"),
    ] {
        let mut inputs = Inputs::default();
        match field {
            0 => inputs.range.as_mut().unwrap().state = RangeState::NoRange,
            1 => {
                inputs
                    .range
                    .as_mut()
                    .unwrap()
                    .range
                    .as_mut()
                    .unwrap()
                    .width_pct = NonNegative::new(50.0).unwrap()
            }
            2 => inputs.range.as_mut().unwrap().state = RangeState::Candidate,
            3 => inputs.range.as_mut().unwrap().confidence = UnitInterval::new(0.1).unwrap(),
            4 => {
                inputs
                    .range
                    .as_mut()
                    .unwrap()
                    .range
                    .as_mut()
                    .unwrap()
                    .latest_inside_bounds = false
            }
            5 => {
                inputs.volume.as_mut().unwrap().activity_fraction = UnitInterval::new(0.0).unwrap()
            }
            _ => {
                inputs.volume.as_mut().unwrap().relative_volume =
                    Some(NonNegative::new(0.1).unwrap())
            }
        }

        let decision = inputs.evaluate(&strategy).unwrap();
        assert_eq!(decision.verdict, verdict);
        assert!(decision.reasons.iter().any(|reason| reason.code == code));
    }
}

#[test]
fn token_filters_distinguish_missing_stale_future_outside_and_accepted_metrics() {
    let mut config = config();
    config.filters.insert(
        TokenMetric::MarketCapUsd,
        MetricBounds {
            gt: Some(100.0),
            ..Default::default()
        },
    );
    let strategy = build_strategy(config);
    let mut candidate = market::candidate();
    let now = market::timestamp(600_000);
    assert!(strategy.uses_token_metrics());
    assert_eq!(strategy.discovery_token_filters().len(), 1);
    assert!(matches!(
        strategy.screen_token_metrics(&candidate, now),
        MetricScreen::Unavailable(_)
    ));
    for (value, fetched, expected) in [
        (200.0, 539_999, "METRIC_STALE"),
        (200.0, 600_001, "METRIC_FUTURE_TIMESTAMP"),
        (50.0, 600_000, "METRIC_OUTSIDE_FILTER"),
    ] {
        let mut observation = market::observation(value);
        observation.fetched_at = market::timestamp(fetched);
        observation.observed_at = None;
        candidate
            .metrics
            .insert(TokenMetric::MarketCapUsd, observation);
        let reasons = match strategy.screen_token_metrics(&candidate, now) {
            MetricScreen::Reject(reasons) | MetricScreen::Unavailable(reasons) => reasons,
            _ => panic!("expected failed filter"),
        };
        assert_eq!(reasons[0].code, expected);
    }

    candidate
        .metrics
        .insert(TokenMetric::MarketCapUsd, market::observation(200.0));
    assert!(matches!(
        strategy.screen_token_metrics(&candidate, now),
        MetricScreen::Continue
    ));
}

#[test]
fn research_chain_prefilter_and_required_trend_are_explicit() {
    let mut config = config();
    config.research = Some(ResearchPolicy {
        allowed_chains: vec![ChainId::Evm { chain_id: 2 }],
        ..Default::default()
    });
    config.trend = Some(TrendBinding {
        analyzer: InstanceId::new("trend").unwrap(),
        required: true,
        reject_downtrend: true,
    });
    let strategy = build_strategy(config);
    assert_eq!(strategy.requirements().analyses.len(), 3);
    let inputs = Inputs::default();
    assert!(matches!(
        strategy.prefilter(&inputs.candidate, market::timestamp(600_000)),
        PrefilterDecision::Reject(_)
    ));
    let decision = inputs.evaluate(&strategy).unwrap();
    assert_eq!(decision.verdict, Verdict::Unavailable);
    assert!(
        decision
            .warnings
            .iter()
            .any(|warning| warning == "TrendUnavailable")
    );
}

#[test]
fn pump_scheduling_waits_for_the_next_closed_candle_plus_grace() {
    let mut config = config();
    config.signal = Some(PumpSignal::default());
    let strategy = build_strategy(config);
    for (now, expected) in [(64_999, 65_000), (65_000, 125_000), (125_000, 185_000)] {
        assert_eq!(
            strategy
                .next_market_check(market::timestamp(now), 5000)
                .unwrap(),
            market::timestamp(expected)
        );
    }

    assert!(
        strategy
            .next_market_check(market::timestamp(600_000), u64::MAX)
            .is_none()
    );
    assert_eq!(strategy.requirements().candles.min_bars, 20);
}

#[test]
fn research_policy_checks_price_linear_position_consistency_and_allowed_zone() {
    let mut config = config();
    config.research = Some(ResearchPolicy::default());
    let strategy = build_strategy(config);
    let mut inputs = Inputs::default();
    inputs.volume.as_mut().unwrap().robust = Some(Box::new(RobustVolumeAnalysis {
        baseline_median: NonNegative::new(1.0).unwrap(),
        recent_median: NonNegative::new(2.0).unwrap(),
        relative_volume: Some(NonNegative::new(2.0).unwrap()),
        short_slope: 0.1,
        long_slope: 0.1,
        state: ActivityState::Elevated,
    }));
    assert_eq!(inputs.evaluate(&strategy).unwrap().verdict, Verdict::Match);
    inputs
        .range
        .as_mut()
        .unwrap()
        .range
        .as_mut()
        .unwrap()
        .position = None;
    assert_eq!(
        inputs.evaluate(&strategy).unwrap().reasons[0].code,
        "POSITION_UNAVAILABLE"
    );
    let mut position = RangePosition::new(1.0, 2.0, 1.5).unwrap();
    position.raw_pct = 51.0;
    inputs
        .range
        .as_mut()
        .unwrap()
        .range
        .as_mut()
        .unwrap()
        .position = Some(Box::new(position));
    assert_eq!(
        inputs.evaluate(&strategy).unwrap().reasons[0].code,
        "POSITION_INVALID"
    );
    inputs
        .range
        .as_mut()
        .unwrap()
        .range
        .as_mut()
        .unwrap()
        .position = Some(Box::new(RangePosition::new(1.0, 2.0, 1.8).unwrap()));
    assert_eq!(
        inputs.evaluate(&strategy).unwrap().reasons[0].code,
        "POSITION_OUTSIDE_POLICY"
    );
}

#[test]
fn public_pump_evaluation_requires_matching_closed_usd_series() {
    let signal = PumpSignal::default();
    let mut series = market::series(&[1.0, 1.02, 1.05, 1.08, 1.12, 1.2], &[1.0; 6]);
    let mut candles = series.candles.to_vec();
    candles[5] = Candle::new(
        market::timestamp(300_000),
        1.12,
        1.2,
        1.12,
        1.2,
        Knowledge::known(NonNegative::new(1.0).unwrap()),
    )
    .unwrap();
    series.candles = candles.into();
    let evaluate = |series: Option<&CandleSeries>, pool: &PoolId, token: &TokenId, as_of| {
        signal.evaluate(series, pool, token, as_of)
    };
    assert_eq!(
        evaluate(
            None,
            &series.pool,
            &series.priced_token,
            series.end_exclusive
        )
        .1[0]
            .code,
        "PUMP_CANDLES_MISSING"
    );
    assert_eq!(
        evaluate(
            Some(&series),
            &series.pool,
            &series.priced_token,
            series.end_exclusive
        )
        .0,
        Verdict::Match
    );
    assert_eq!(
        evaluate(
            Some(&series),
            &market::pool_id(99),
            &series.priced_token,
            series.end_exclusive
        )
        .0,
        Verdict::Unavailable
    );
    assert_eq!(
        evaluate(
            Some(&series),
            &series.pool,
            &market::token(99),
            series.end_exclusive
        )
        .0,
        Verdict::Unavailable
    );
    assert_eq!(
        evaluate(
            Some(&series),
            &series.pool,
            &series.priced_token,
            market::timestamp(0)
        )
        .0,
        Verdict::Unavailable
    );
}
