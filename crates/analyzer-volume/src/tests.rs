use super::*;
use analysis_api::*;
use analysis_model::ActivityState;
use domain::*;
use test_support::market as fixtures;

fn build_analyzer(robust: bool) -> VolumeAnalyzer {
    VolumeAnalyzer::new(
        InstanceId::new("volume").unwrap(),
        VolumeConfig {
            recent_bars: 2,
            baseline_bars: 2,
            robust: robust.then_some(RobustVolumeConfig {
                short_bars: 2,
                long_bars: 4,
            }),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn configuration_validates_windows_and_robust_history() {
    assert!(RobustVolumeConfig::default().validate(60).is_ok());
    for config in [
        RobustVolumeConfig {
            short_bars: 1,
            long_bars: 4,
        },
        RobustVolumeConfig {
            short_bars: 4,
            long_bars: 2,
        },
        RobustVolumeConfig {
            short_bars: 2,
            long_bars: 5,
        },
    ] {
        assert!(config.validate(4).is_err());
    }

    for (recent_bars, baseline_bars) in [(0, 2), (2, 0), (5001, 2), (2, 5001)] {
        assert!(
            VolumeAnalyzer::new(
                InstanceId::new("volume").unwrap(),
                VolumeConfig {
                    recent_bars,
                    baseline_bars,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }

    assert!(
        VolumeAnalyzer::new(
            InstanceId::new("volume").unwrap(),
            VolumeConfig {
                recent_bars: 1,
                baseline_bars: 1,
                robust: Some(RobustVolumeConfig::default()),
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn mean_and_robust_volume_classify_inactive_unknown_normal_and_elevated() {
    for robust in [false, true] {
        let analyzer = build_analyzer(robust);
        for (values, state, ratio) in [
            ([1.0, 1.0, 0.0, 0.0], ActivityState::Inactive, Some(0.0)),
            ([0.0, 0.0, 1.0, 1.0], ActivityState::Unknown, None),
            ([2.0, 2.0, 2.0, 2.0], ActivityState::Normal, Some(1.0)),
            ([2.0, 2.0, 4.0, 4.0], ActivityState::Elevated, Some(2.0)),
        ] {
            let result = analyzer.analyze_metrics(&values, VolumeUnit::Usd).unwrap();
            assert_eq!(result.state, state);
            assert_eq!(result.relative_volume.map(NonNegative::get), ratio);
            assert_eq!(
                result.activity_fraction.get(),
                if values[3] == 0.0 { 0.0 } else { 1.0 }
            );
            if robust {
                let robust = result.robust.unwrap();
                assert_eq!(robust.state, state);
                assert_eq!(robust.relative_volume.map(NonNegative::get), ratio);
            } else {
                assert!(result.robust.is_none());
            }
        }
    }
}

#[test]
fn metrics_use_latest_window_and_reject_invalid_inputs_and_overflow() {
    let analyzer = build_analyzer(false);
    let result = analyzer
        .analyze_metrics(&[f64::NAN, 1.0, 3.0, 2.0, 4.0], VolumeUnit::Usd)
        .unwrap();
    assert_eq!(result.baseline_mean.get(), 2.0);
    assert_eq!(result.recent_mean.get(), 3.0);
    assert_eq!(result.last_volume_percentile.get(), 1.0);
    assert_eq!(
        analyzer
            .analyze_metrics(&[1.0; 4], VolumeUnit::Usd)
            .unwrap()
            .last_volume_percentile
            .get(),
        0.5
    );
    assert!(matches!(
        analyzer.analyze_metrics(&[1.0; 3], VolumeUnit::Usd),
        Err(AnalysisError::InsufficientHistory { .. })
    ));
    assert!(matches!(
        analyzer.analyze_metrics(&[1.0; 4], VolumeUnit::Unknown),
        Err(AnalysisError::UnsupportedUnits)
    ));
    for invalid in [-1.0, f64::NAN, f64::INFINITY, f64::MAX] {
        assert!(
            analyzer
                .analyze_metrics(&[invalid; 4], VolumeUnit::Usd)
                .is_err()
        );
    }
}

#[test]
fn analyzer_contract_validates_context_and_requires_known_volume() {
    let analyzer = build_analyzer(true);
    assert_eq!(analyzer.instance().as_str(), "volume");
    assert_eq!(
        analyzer.descriptor().id,
        VolumeAnalyzer::descriptor_static().id
    );
    assert_eq!(
        analyzer.config_digest(),
        build_analyzer(true).config_digest()
    );
    let requirements = analyzer.requirements(Resolution::new(60).unwrap(), PriceUnit::UsdPerToken);
    assert_eq!(requirements.candles.min_bars, 4);
    assert_eq!(requirements.candles.desired_bars, 4);
    assert!(requirements.candles.require_volume);
    let mut series = fixtures::series(&[1.0; 4], &[1.0, 2.0, 3.0, 4.0]);

    let analyze = |series: &CandleSeries, as_of| {
        analyzer.analyze(&AnalysisContext {
            candles: series,
            as_of,
        })
    };
    assert!(matches!(
        analyze(&series, series.end_exclusive),
        Ok(AnalysisValue::Volume(_))
    ));
    assert!(analyze(&series, fixtures::timestamp(0)).is_err());
    let mut candles = series.candles.to_vec();
    candles[0].volume = Knowledge::unknown();
    series.candles = candles.into();
    assert!(matches!(
        analyze(&series, series.end_exclusive),
        Err(AnalysisError::MissingDependency)
    ));
    series.end_exclusive = fixtures::timestamp(240_001);
    assert!(analyze(&series, series.end_exclusive).is_err());
}
