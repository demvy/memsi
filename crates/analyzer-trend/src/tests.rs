use super::*;
#[path = "../../../test-support/market.rs"]
mod fixtures;

fn build_analyzer() -> TrendAnalyzer {
    TrendAnalyzer::new(
        InstanceId::new("trend").unwrap(),
        TrendConfig {
            window: 4,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn constructor_checks_each_configuration_bound() {
    for config in [
        TrendConfig {
            window: 1,
            ..Default::default()
        },
        TrendConfig {
            window: 1501,
            ..Default::default()
        },
        TrendConfig {
            neutral_slope_per_bar: -1.0,
            ..Default::default()
        },
        TrendConfig {
            neutral_slope_per_bar: f64::NAN,
            ..Default::default()
        },
        TrendConfig {
            min_r_squared: -0.1,
            ..Default::default()
        },
        TrendConfig {
            min_r_squared: 1.1,
            ..Default::default()
        },
        TrendConfig {
            min_r_squared: f64::NAN,
            ..Default::default()
        },
    ] {
        assert!(TrendAnalyzer::new(InstanceId::new("trend").unwrap(), config).is_err());
    }
}

#[test]
fn trend_reports_log_linear_slopes_directions_and_scale_invariance() {
    let analyzer = build_analyzer();
    let analyze = |prices: &[f64]| {
        analyzer
            .analyze_prices(prices, Resolution::new(60).unwrap(), PriceUnit::UsdPerToken)
            .unwrap()
    };
    let up = analyze(&[1.0, 2.0, 4.0, 8.0]);
    assert_eq!(up.direction, TrendDirection::Up);
    assert!((up.slope_log_per_bar - 2f64.ln()).abs() < 1e-12);
    assert!((up.r_squared.get() - 1.0).abs() < 1e-12);
    let scaled = analyze(&[10.0, 20.0, 40.0, 80.0]);
    assert!((scaled.slope_log_per_bar - up.slope_log_per_bar).abs() < 1e-12);
    assert_eq!(
        analyze(&[8.0, 4.0, 2.0, 1.0]).direction,
        TrendDirection::Down
    );
    let flat = analyze(&[2.0; 4]);
    assert_eq!(flat.direction, TrendDirection::Sideways);
    assert_eq!(flat.r_squared.get(), 0.0);
    assert_eq!(
        analyze(&[f64::NAN, 1.0, 2.0, 4.0, 8.0]).direction,
        TrendDirection::Up
    );
}

#[test]
fn price_analysis_rejects_short_and_invalid_windows() {
    let analyzer = build_analyzer();
    let resolution = Resolution::new(60).unwrap();
    assert!(matches!(
        analyzer.analyze_prices(&[1.0; 3], resolution, PriceUnit::UsdPerToken),
        Err(AnalysisError::InsufficientHistory { have: 3, need: 4 })
    ));
    for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            analyzer
                .analyze_prices(
                    &[1.0, invalid, 2.0, 3.0],
                    resolution,
                    PriceUnit::UsdPerToken
                )
                .is_err()
        );
    }
}

#[test]
fn analyzer_contract_and_context_check_cutoff_history_and_alignment() {
    let analyzer = build_analyzer();
    assert_eq!(analyzer.instance().as_str(), "trend");
    assert_eq!(
        analyzer.descriptor().id,
        TrendAnalyzer::descriptor_static().id
    );
    assert_eq!(analyzer.config_digest(), build_analyzer().config_digest());
    assert!(!analyzer.stateful());
    let requirements = analyzer.requirements(Resolution::new(60).unwrap(), PriceUnit::UsdPerToken);
    assert_eq!(requirements.candles.min_bars, 4);
    assert_eq!(requirements.candles.desired_bars, 4);
    assert!(!requirements.candles.require_volume);
    assert!(requirements.dependencies.is_empty());
    let mut series = fixtures::series(&[1.0, 2.0, 4.0, 8.0], &[1.0; 4]);
    let dependencies = ArtifactStore::default();
    let analyze = |series: &CandleSeries, as_of| {
        analyzer.analyze(&AnalysisContext {
            candles: series,
            dependencies: &dependencies,
            as_of,
        })
    };
    assert!(matches!(
        analyze(&series, series.end_exclusive),
        Ok(AnalysisValue::Trend(_))
    ));
    assert!(analyze(&series, fixtures::timestamp(0)).is_err());
    series.end_exclusive = fixtures::timestamp(240_001);
    assert!(analyze(&series, series.end_exclusive).is_err());
    series.candles = vec![series.candles[0].clone()].into();
    assert!(matches!(
        analyze(&series, series.end_exclusive),
        Err(AnalysisError::InsufficientHistory { .. })
    ));
}
