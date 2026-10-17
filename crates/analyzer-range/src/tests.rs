use super::*;
use analysis_api::*;
use domain::*;
#[path = "../../../test-support/market.rs"]
mod fixtures;

#[test]
fn constructor_normalizes_windows_and_checks_resource_and_history_bounds() {
    let instance = InstanceId::new("range").unwrap();
    let config = RangeConfig::default();
    assert!(RangeAnalyzer::new(instance.clone(), config.clone()).is_ok());
    for field in 0..7 {
        let mut invalid = config.clone();
        match field {
            0 => invalid.windows.clear(),
            1 => invalid.windows = vec![20; 65],
            2 => invalid.windows = vec![10001],
            3 => invalid.occupancy_bins = 1025,
            4 => invalid.min_bars = 10001,
            5 => invalid.lower_quantile = 0.6,
            _ => invalid.history_bars = Some(10001),
        }

        assert!(RangeAnalyzer::new(instance.clone(), invalid).is_err());
    }

    let mut reordered = config.clone();
    reordered.windows.reverse();
    reordered.windows.push(reordered.windows[0]);
    assert_eq!(
        RangeAnalyzer::new(instance.clone(), config)
            .unwrap()
            .config_digest(),
        RangeAnalyzer::new(instance, reordered)
            .unwrap()
            .config_digest()
    );
}

#[test]
fn analyzer_contract_runs_detector_and_rejects_future_and_short_history() {
    let analyzer =
        RangeAnalyzer::new(InstanceId::new("range").unwrap(), RangeConfig::default()).unwrap();
    assert_eq!(analyzer.instance().as_str(), "range");
    assert_eq!(
        analyzer.descriptor().id,
        RangeAnalyzer::descriptor_static().id
    );
    let requirements = analyzer.requirements(Resolution::new(60).unwrap(), PriceUnit::UsdPerToken);
    assert!(!requirements.candles.require_volume);
    assert!(requirements.dependencies.is_empty());
    let count = requirements.candles.desired_bars;
    let prices: Vec<_> = (0..count)
        .map(|index| 1.5 + (index as f64 * 0.3).sin() * 0.1)
        .collect();
    let mut series = fixtures::series(&prices, &vec![1.0; count]);
    let dependencies = ArtifactStore::default();
    let analyze = |series: &CandleSeries, as_of| {
        analyzer.analyze(&AnalysisContext {
            candles: series,
            dependencies: &dependencies,
            as_of,
        })
    };
    let AnalysisValue::Range(result) = analyze(&series, series.end_exclusive).unwrap() else {
        panic!("expected range analysis")
    };
    assert_eq!(result.price_unit, PriceUnit::UsdPerToken);
    if let Some(bounds) = result.range {
        assert!(bounds.position.is_some());
    }
    assert!(analyze(&series, fixtures::timestamp(0)).is_err());
    series.candles = vec![series.candles[0].clone()].into();
    assert!(analyze(&series, series.end_exclusive).is_err());
}

#[test]
fn detector_error_messages_explain_invalid_inputs_and_scratch_can_be_reused() {
    use crate::detector_core::{
        RangeDetector, RangeDetectorConfig, RangeDetectorScratch, RangeError,
    };
    for (error, message) in [
        (RangeError::EmptyInput, "input is empty"),
        (
            RangeError::InvalidConfig("window"),
            "invalid detector config: window",
        ),
        (
            RangeError::NonPositivePrice {
                index: 2,
                value: 0.0,
            },
            "price at index 2 is not positive: 0",
        ),
        (
            RangeError::NonFinitePrice {
                index: 3,
                value: f64::INFINITY,
            },
            "price at index 3 is not finite: inf",
        ),
    ] {
        assert_eq!(error.to_string(), message);
    }

    let mut scratch = RangeDetectorScratch::new();
    let detector = RangeDetector::new(RangeDetectorConfig::default()).unwrap();
    let first = detector
        .detect_closes_with_scratch(&vec![1.0; 300], &mut scratch)
        .unwrap();
    let second = detector
        .detect_closes_with_scratch(&vec![2.0; 300], &mut scratch)
        .unwrap();
    assert_eq!(first.state, second.state);
}

#[test]
fn candle_detector_and_trait_adapter_use_the_same_close_prices() {
    use crate::detector_core::{Candle, DetectRange, RangeDetector, RangeDetectorConfig};
    let detector = RangeDetector::new(RangeDetectorConfig::default()).unwrap();
    let candles: Vec<_> = (0..300)
        .map(|index| Candle {
            timestamp_ms: index * 60_000,
            open: 1.0,
            high: 2.0,
            low: 1.0,
            close: 1.5,
            volume: 10.0,
        })
        .collect();
    let direct = detector.detect_candles(&candles).unwrap();
    let adapter: &dyn DetectRange = &detector;
    let through_trait = adapter.detect_closes(&vec![1.5; 300]).unwrap();
    assert_eq!(direct.state, through_trait.state);
    assert_eq!(direct.confidence, through_trait.confidence);
}
