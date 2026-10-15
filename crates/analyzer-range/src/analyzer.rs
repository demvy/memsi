use crate::RangeConfig;
use analysis_api::*;
use analysis_model::{RangeAnalysis, RangeBounds, RangeState};
use domain::*;
pub struct RangeAnalyzer {
    instance: InstanceId,
    config_digest: ContentHash,
    detector: crate::detector_core::RangeDetector,
    window: usize,
    desired_bars: usize,
}
impl RangeAnalyzer {
    pub fn new(instance: InstanceId, mut config: RangeConfig) -> Result<Self, AnalysisError> {
        if config.windows.is_empty()
            || config.windows.len() > 64
            || config.windows.iter().any(|w| *w > 10000)
            || config.occupancy_bins > 1024
        {
            return Err(AnalysisError::InvalidInput("range resource bounds".into()));
        }
        config.windows.sort_unstable();
        config.windows.dedup();
        let window = *config.windows.last().unwrap();
        if config.min_bars > window || config.lower_quantile > 0.5 || config.upper_quantile < 0.5 {
            return Err(AnalysisError::InvalidInput(
                "unusable range window/quantiles".into(),
            ));
        }
        let desired_bars = config.history_bars.unwrap_or(window.max(300));
        if desired_bars < window || desired_bars > 10000 {
            return Err(AnalysisError::InvalidInput("range history bounds".into()));
        }
        let detector = crate::detector_core::RangeDetector::new(config.clone().into())
            .map_err(|e| AnalysisError::InvalidInput(e.to_string()))?;
        let config_digest = ContentHash::of(&config).map_err(|_| AnalysisError::NumericFailure)?;
        Ok(Self {
            instance,
            config_digest,
            detector,
            window,
            desired_bars,
        })
    }
    pub fn descriptor_static() -> plugin_api::PluginDescriptor {
        plugin_api::PluginDescriptor {
            id: PluginId::new("analyzer.range").unwrap(),
            version: env!("CARGO_PKG_VERSION").into(),
            kind: plugin_api::PluginKind::Analyzer,
            provides: vec![plugin_api::Capability::pure(
                CapabilityId::new("analysis.range").unwrap(),
                1,
            )],
        }
    }
}
impl Analyzer for RangeAnalyzer {
    fn descriptor(&self) -> plugin_api::PluginDescriptor {
        Self::descriptor_static()
    }
    fn instance(&self) -> &InstanceId {
        &self.instance
    }
    fn config_digest(&self) -> &ContentHash {
        &self.config_digest
    }
    fn requirements(&self, resolution: Resolution, price_unit: PriceUnit) -> AnalysisRequirements {
        AnalysisRequirements {
            candles: CandleRequirement {
                resolution,
                price_unit,
                min_bars: self.window,
                desired_bars: self.desired_bars,
                require_volume: false,
            },
            dependencies: vec![],
        }
    }
    fn analyze(&self, ctx: &AnalysisContext<'_>) -> Result<AnalysisValue, AnalysisError> {
        if ctx.candles.end_exclusive > ctx.as_of {
            return Err(AnalysisError::InvalidInput("future candle cutoff".into()));
        }
        ctx.candles
            .validate(self.window)
            .map_err(|e| AnalysisError::InvalidInput(e.to_string()))?;
        let candles = &ctx.candles.candles;
        let closes: Vec<f64> = candles[candles.len() - self.window..]
            .iter()
            .map(|c| c.close.get())
            .collect();
        let result = self
            .detector
            .detect_closes(&closes)
            .map_err(|e| AnalysisError::InvalidInput(e.to_string()))?;
        let state = match result.state {
            crate::detector_core::RangeState::InsufficientData => RangeState::InsufficientData,
            crate::detector_core::RangeState::NoRange => RangeState::NoRange,
            crate::detector_core::RangeState::RangeCandidate => RangeState::Candidate,
            crate::detector_core::RangeState::RangeConfirmed => RangeState::Confirmed,
        };
        let range = result
            .range
            .map(|r| -> Result<RangeBounds, AnalysisError> {
                Ok(RangeBounds {
                    lower: PositivePrice::new(r.lower)
                        .map_err(|_| AnalysisError::NumericFailure)?,
                    upper: PositivePrice::new(r.upper)
                        .map_err(|_| AnalysisError::NumericFailure)?,
                    center: PositivePrice::new(r.center)
                        .map_err(|_| AnalysisError::NumericFailure)?,
                    width_pct: NonNegative::new(r.width_pct * 100.0)
                        .map_err(|_| AnalysisError::NumericFailure)?,
                    age_bars: r.age_bars,
                    latest_position: r.latest_position,
                    position: Some(Box::new(
                        analysis_model::RangePosition::new(
                            r.lower,
                            r.upper,
                            *closes.last().unwrap(),
                        )
                        .ok_or(AnalysisError::NumericFailure)?,
                    )),
                    latest_inside: r.latest_inside,
                    latest_inside_bounds: r.lower <= *closes.last().unwrap()
                        && *closes.last().unwrap() <= r.upper,
                    outside_streak: r.outside_streak,
                })
            })
            .transpose()?;
        Ok(AnalysisValue::Range(RangeAnalysis {
            state,
            range,
            confidence: UnitInterval::new(result.confidence)
                .map_err(|_| AnalysisError::NumericFailure)?,
            price_unit: ctx.candles.price_unit.clone(),
        }))
    }
}
