#![forbid(unsafe_code)]
use analysis_api::*;
use analysis_model::{TrendAnalysis, TrendDirection};
use domain::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrendConfig {
    pub window: usize,
    pub neutral_slope_per_bar: f64,
    pub min_r_squared: f64,
}

impl Default for TrendConfig {
    fn default() -> Self {
        Self {
            window: 120,
            neutral_slope_per_bar: 0.0005,
            min_r_squared: 0.20,
        }
    }
}

pub struct TrendAnalyzer {
    instance: InstanceId,
    config: TrendConfig,
    digest: ContentHash,
}

impl TrendAnalyzer {
    pub fn new(instance: InstanceId, config: TrendConfig) -> Result<Self, AnalysisError> {
        if !(2..=1500).contains(&config.window)
            || !config.neutral_slope_per_bar.is_finite()
            || config.neutral_slope_per_bar < 0.
            || !config.min_r_squared.is_finite()
            || !(0.0..=1.0).contains(&config.min_r_squared)
        {
            return Err(AnalysisError::InvalidInput("trend configuration".into()));
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
            id: PluginId::new("analyzer.trend").unwrap(),
            version: env!("CARGO_PKG_VERSION").into(),
            kind: plugin_api::PluginKind::Analyzer,
            provides: vec![plugin_api::Capability::pure(
                CapabilityId::new("analysis.trend").unwrap(),
                1,
            )],
        }
    }

    /// OLS on the latest closed window. Multiplicative price units do not change log slope.
    pub fn analyze_prices(
        &self,
        prices: &[f64],
        resolution: Resolution,
        price_unit: PriceUnit,
    ) -> Result<TrendAnalysis, AnalysisError> {
        if prices.len() < self.config.window {
            return Err(AnalysisError::InsufficientHistory {
                have: prices.len(),
                need: self.config.window,
            });
        }

        let prices = &prices[prices.len() - self.config.window..];
        if prices
            .iter()
            .any(|price| !price.is_finite() || *price <= 0.)
        {
            return Err(AnalysisError::InvalidInput(
                "trend prices must be positive and finite".into(),
            ));
        }

        let origin = prices[0].ln();
        let logs: Vec<_> = prices.iter().map(|price| price.ln() - origin).collect();
        let mean = sum(logs.iter().copied()) / logs.len() as f64;
        let center_x = (logs.len() - 1) as f64 / 2.;
        let centered: Vec<_> = logs
            .iter()
            .enumerate()
            .map(|(i, log)| (i as f64 - center_x, log - mean))
            .collect();
        let xx = sum(centered.iter().map(|(x, _)| x * x));
        let xy = sum(centered.iter().map(|(x, y)| x * y));
        let slope = xy / xx;
        let total = sum(centered.iter().map(|(_, y)| y * y));
        let residual = sum(centered.iter().map(|(x, y)| (y - slope * x).powi(2)));
        let r_squared = if total == 0. {
            0.
        } else {
            (1. - residual / total).clamp(0., 1.)
        };

        if !slope.is_finite() || !r_squared.is_finite() {
            return Err(AnalysisError::NumericFailure);
        }

        let direction = if r_squared < self.config.min_r_squared
            || slope.abs() <= self.config.neutral_slope_per_bar
        {
            TrendDirection::Sideways
        } else if slope > 0. {
            TrendDirection::Up
        } else {
            TrendDirection::Down
        };

        Ok(TrendAnalysis {
            slope_log_per_bar: slope,
            r_squared: UnitInterval::new(r_squared).map_err(|_| AnalysisError::NumericFailure)?,
            direction,
            window: logs.len(),
            resolution,
            price_unit,
        })
    }
}

fn sum(values: impl Iterator<Item = f64>) -> f64 {
    let mut total = 0.;
    let mut correction = 0.;
    for value in values {
        let adjusted = value - correction;
        let next = total + adjusted;
        correction = (next - total) - adjusted;
        total = next;
    }

    total
}

impl Analyzer for TrendAnalyzer {
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
        AnalysisRequirements {
            candles: CandleRequirement {
                resolution,
                price_unit,
                min_bars: self.config.window,
                desired_bars: self.config.window,
                require_volume: false,
            },
            dependencies: vec![],
        }
    }

    fn analyze(&self, ctx: &AnalysisContext<'_>) -> Result<AnalysisValue, AnalysisError> {
        if ctx.candles.end_exclusive > ctx.as_of {
            return Err(AnalysisError::InvalidInput("future candle cutoff".into()));
        }

        if ctx.candles.candles.len() < self.config.window {
            return Err(AnalysisError::InsufficientHistory {
                have: ctx.candles.candles.len(),
                need: self.config.window,
            });
        }

        ctx.candles
            .validate(self.config.window)
            .map_err(|error| AnalysisError::InvalidInput(error.to_string()))?;
        let prices: Vec<_> = ctx.candles.candles[ctx.candles.candles.len() - self.config.window..]
            .iter()
            .map(|candle| candle.close.get())
            .collect();
        self.analyze_prices(
            &prices,
            ctx.candles.resolution,
            ctx.candles.price_unit.clone(),
        )
        .map(AnalysisValue::Trend)
    }
}

#[cfg(test)]
mod tests;
