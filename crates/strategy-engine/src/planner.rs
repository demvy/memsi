use analysis_api::{Analyzer, CandleRequirement};
use domain::*;
use std::collections::BTreeMap;
use strategy_api::Strategy;
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("strategy {0} needs required range/volume with optional trend")]
    Bindings(InstanceId),
    #[error("analyzer {0} is missing or provides an incompatible schema")]
    Analyzer(InstanceId),
    #[error("nested analyzer dependencies are not supported in the range/volume planner")]
    NestedDependencies,
    #[error("incompatible candle requirements")]
    Candles,
}

#[derive(Clone)]
pub struct StrategyPlan<'a> {
    pub strategy: &'a dyn Strategy,
    pub range: Option<&'a dyn Analyzer>,
    pub volume: Option<&'a dyn Analyzer>,
    pub trend: Option<&'a dyn Analyzer>,
    pub candles: CandleRequirement,
}

/// Validate bindings before any market-data request or task starts.
pub fn plan<'a>(
    strategy: &'a dyn Strategy,
    analyzers: &'a BTreeMap<InstanceId, Box<dyn Analyzer>>,
) -> Result<StrategyPlan<'a>, PlanError> {
    let requirements = strategy.requirements();
    if !(2..=3).contains(&requirements.analyses.len())
        || requirements
            .analyses
            .iter()
            .any(|binding| !["range", "volume", "trend"].contains(&binding.name.as_str()))
        || requirements
            .analyses
            .iter()
            .enumerate()
            .any(|(i, binding)| {
                requirements.analyses[..i]
                    .iter()
                    .any(|other| other.name == binding.name)
            })
    {
        return Err(PlanError::Bindings(strategy.instance().clone()));
    }

    let find = |name: &str, schema: &str| -> Result<&'a dyn Analyzer, PlanError> {
        let binding = requirements
            .analyses
            .iter()
            .find(|binding| binding.name.as_str() == name)
            .ok_or_else(|| PlanError::Bindings(strategy.instance().clone()))?;
        if (name != "trend" && !binding.required)
            || binding.major != 1
            || binding.schema.as_str() != schema
        {
            return Err(PlanError::Bindings(strategy.instance().clone()));
        }

        let analyzer = analyzers
            .get(&binding.instance)
            .ok_or_else(|| PlanError::Analyzer(binding.instance.clone()))?;
        if !analyzer
            .descriptor()
            .provides
            .iter()
            .any(|capability| capability.id.as_str() == schema && capability.schema_major == 1)
        {
            return Err(PlanError::Analyzer(binding.instance.clone()));
        }

        Ok(analyzer.as_ref())
    };

    let range = Some(find("range", "analysis.range")?);
    let volume = Some(find("volume", "analysis.volume")?);
    let trend = if requirements
        .analyses
        .iter()
        .any(|binding| binding.name.as_str() == "trend")
    {
        Some(find("trend", "analysis.trend")?)
    } else {
        None
    };

    let mut candles = requirements.candles;
    validate_candles(&candles)?;
    let trend_required = requirements
        .analyses
        .iter()
        .any(|binding| binding.name.as_str() == "trend" && binding.required);
    for (analyzer, mandatory) in [(range, true), (volume, true), (trend, trend_required)] {
        let Some(analyzer) = analyzer else { continue };
        let required = analyzer.requirements(candles.resolution, candles.price_unit.clone());
        if !required.dependencies.is_empty() {
            return Err(PlanError::NestedDependencies);
        }

        validate_candles(&required.candles)?;
        if required.candles.resolution != candles.resolution
            || required.candles.price_unit != candles.price_unit
        {
            return Err(PlanError::Candles);
        }

        if mandatory {
            candles.min_bars = candles.min_bars.max(required.candles.min_bars);
            candles.require_volume |= required.candles.require_volume;
        }

        candles.desired_bars = candles.desired_bars.max(required.candles.desired_bars);
    }

    Ok(StrategyPlan {
        strategy,
        range,
        volume,
        trend,
        candles,
    })
}

fn validate_candles(candles: &CandleRequirement) -> Result<(), PlanError> {
    if candles.min_bars == 0
        || candles.min_bars > candles.desired_bars
        || candles.desired_bars > 1500
        || candles.price_unit != PriceUnit::UsdPerToken
    {
        return Err(PlanError::Candles);
    }

    Ok(())
}

pub fn union_windows(plans: &[StrategyPlan<'_>]) -> BTreeMap<Resolution, usize> {
    let mut windows = BTreeMap::new();
    for plan in plans {
        let window = windows.entry(plan.candles.resolution).or_insert(0);
        *window = (*window).max(plan.candles.desired_bars);
    }

    windows
}
