use crate::VolumeConfig;
use crate::robust;
use analysis_api::identifiers;
use analysis_api::*;
use analysis_model::{ActivityState, VolumeAnalysis};
use domain::*;
pub struct VolumeAnalyzer {
    instance: InstanceId,
    config: VolumeConfig,
    digest: ContentHash,
}

impl VolumeAnalyzer {
    pub fn new(instance: InstanceId, config: VolumeConfig) -> Result<Self, AnalysisError> {
        if config.recent_bars == 0
            || config.baseline_bars == 0
            || config.recent_bars > 5000
            || config.baseline_bars > 5000
        {
            return Err(AnalysisError::InvalidInput("volume windows".into()));
        }

        if let Some(robust) = &config.robust {
            robust.validate(config.recent_bars + config.baseline_bars)?;
        }

        let digest = ContentHash::of(&config).map_err(|_| AnalysisError::NumericFailure)?;
        Ok(Self {
            instance,
            config,
            digest,
        })
    }

    pub fn descriptor_static() -> plugin_api::PluginDescriptor {
        plugin_api::PluginDescriptor {
            id: PluginId::new("analyzer.volume").unwrap(),
            version: env!("CARGO_PKG_VERSION").into(),
            kind: plugin_api::PluginKind::Analyzer,
            provides: vec![plugin_api::Capability::pure(
                CapabilityId::new(identifiers::VOLUME_SCHEMA).unwrap(),
                identifiers::SCHEMA_MAJOR,
            )],
        }
    }

    pub(crate) fn analyze_metrics(
        &self,
        values: &[f64],
        unit: VolumeUnit,
    ) -> Result<VolumeAnalysis, AnalysisError> {
        let count = self.config.recent_bars + self.config.baseline_bars;
        if values.len() < count {
            return Err(AnalysisError::InsufficientHistory {
                have: values.len(),
                need: count,
            });
        }

        if unit == VolumeUnit::Unknown {
            return Err(AnalysisError::UnsupportedUnits);
        }

        let v = &values[values.len() - count..];
        if v.iter().any(|x| !x.is_finite() || *x < 0.0) {
            return Err(AnalysisError::InvalidInput("volume numbers".into()));
        }

        let (baseline, recent) = v.split_at(self.config.baseline_bars);
        let b = baseline.iter().sum::<f64>() / baseline.len() as f64;
        let r = recent.iter().sum::<f64>() / recent.len() as f64;
        let ratio = if b > 0.0 {
            Some(NonNegative::new(r / b).map_err(|_| AnalysisError::NumericFailure)?)
        } else {
            None
        };

        let activity = recent.iter().filter(|x| **x > 0.0).count() as f64 / recent.len() as f64;
        let last = v[count - 1];
        let reference = &v[..count - 1];
        let percentile = (reference.iter().filter(|x| **x < last).count() as f64
            + 0.5 * reference.iter().filter(|x| **x == last).count() as f64)
            / reference.len() as f64;
        let state = if r == 0.0 {
            ActivityState::Inactive
        } else {
            match ratio {
                None => ActivityState::Unknown,
                Some(x) if x.get() >= self.config.elevated_ratio.get() => ActivityState::Elevated,
                Some(_) => ActivityState::Normal,
            }
        };

        Ok(VolumeAnalysis {
            robust: self
                .config
                .robust
                .as_ref()
                .map(|config| {
                    robust::analyze(config, baseline, recent, self.config.elevated_ratio)
                        .map(Box::new)
                })
                .transpose()?,
            recent_mean: NonNegative::new(r).map_err(|_| AnalysisError::NumericFailure)?,
            baseline_mean: NonNegative::new(b).map_err(|_| AnalysisError::NumericFailure)?,
            relative_volume: ratio,
            activity_fraction: UnitInterval::new(activity).unwrap(),
            last_volume_percentile: UnitInterval::new(percentile).unwrap(),
            unit,
            recent_bars: recent.len(),
            baseline_bars: baseline.len(),
            state,
        })
    }
}

impl Analyzer for VolumeAnalyzer {
    fn descriptor(&self) -> plugin_api::PluginDescriptor {
        Self::descriptor_static()
    }

    fn instance(&self) -> &InstanceId {
        &self.instance
    }

    fn config_digest(&self) -> &ContentHash {
        &self.digest
    }

    fn requirements(&self, resolution: Resolution, price_unit: PriceUnit) -> AnalysisRequirements {
        let n = self.config.recent_bars + self.config.baseline_bars;
        AnalysisRequirements {
            candles: CandleRequirement {
                resolution,
                price_unit,
                min_bars: n,
                desired_bars: n,
                require_volume: true,
            },
        }
    }

    fn analyze(&self, ctx: &AnalysisContext<'_>) -> Result<AnalysisValue, AnalysisError> {
        if ctx.candles.end_exclusive > ctx.as_of {
            return Err(AnalysisError::InvalidInput("future candle cutoff".into()));
        }

        ctx.candles
            .validate(self.config.recent_bars + self.config.baseline_bars)
            .map_err(|e| AnalysisError::InvalidInput(e.to_string()))?;
        let count = self.config.recent_bars + self.config.baseline_bars;
        let bars = &ctx.candles.candles;
        let values: Vec<f64> = bars[bars.len() - count..]
            .iter()
            .map(|c| {
                c.volume
                    .as_ref()
                    .map(|x| x.get())
                    .ok_or(AnalysisError::MissingDependency)
            })
            .collect::<Result<_, _>>()?;
        let result = self.analyze_metrics(&values, ctx.candles.volume_unit.clone())?;
        Ok(AnalysisValue::Volume(result))
    }
}
