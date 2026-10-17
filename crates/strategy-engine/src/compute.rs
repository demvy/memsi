use analysis_api::*;
use domain::*;
use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Mutex,
};

/// Evaluation-local cache. It never mixes snapshots or survives config changes.
#[derive(Default)]
pub(super) struct ComputeCache {
    results: Mutex<BTreeMap<ContentHash, Result<AnalysisValue, AnalysisError>>>,
}

pub(super) struct ComputedAnalysis {
    pub value: AnalysisValue,
    pub input_hash: ContentHash,
}

impl ComputeCache {
    pub fn analyze(
        &self,
        analyzer: &dyn Analyzer,
        series: &CandleSeries,
        as_of: TimestampMs,
    ) -> Result<ComputedAnalysis, AnalysisError> {
        let requirement = analyzer
            .requirements(series.resolution, series.price_unit.clone())
            .candles;
        series
            .validate(requirement.min_bars)
            .map_err(|error| AnalysisError::InvalidInput(error.to_string()))?;
        let count = series.candles.len().min(requirement.desired_bars);
        let mut view = series.clone();
        view.candles = series.candles[series.candles.len() - count..]
            .to_vec()
            .into();
        view.provenance.input_hash = ContentHash::of(&(
            &view.pool,
            &view.priced_token,
            &view.price_unit,
            &view.volume_unit,
            view.resolution,
            view.end_exclusive,
            &view.candles,
        ))
        .map_err(|_| AnalysisError::NumericFailure)?;
        let descriptor = analyzer.descriptor();
        let key = ContentHash::of(&(
            descriptor.id,
            descriptor.version,
            analyzer.stateful().then(|| analyzer.instance()),
            analyzer.config_digest(),
            &view.provenance.input_hash,
            as_of,
        ))
        .map_err(|_| AnalysisError::NumericFailure)?;
        let mut results = self
            .results
            .lock()
            .map_err(|_| AnalysisError::InvalidInput("compute cache unavailable".into()))?;
        if let Some(result) = results.get(&key) {
            return result.clone().map(|value| ComputedAnalysis {
                value,
                input_hash: view.provenance.input_hash,
            });
        }

        let artifacts = ArtifactStore::default();
        let context = AnalysisContext {
            candles: &view,
            dependencies: &artifacts,
            as_of,
        };

        let result = catch_unwind(AssertUnwindSafe(|| analyzer.analyze(&context)))
            .unwrap_or_else(|_| Err(AnalysisError::InvalidInput("analyzer panic".into())));
        results.insert(key, result.clone());
        result.map(|value| ComputedAnalysis {
            value,
            input_hash: view.provenance.input_hash,
        })
    }
}
