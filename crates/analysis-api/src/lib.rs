#![forbid(unsafe_code)]
use analysis_model::Artifact;
use domain::*;
use serde::{Deserialize, Serialize};
use std::{any::Any, collections::BTreeMap, marker::PhantomData, sync::Arc};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArtifactSlot {
    pub schema: SchemaId,
    pub major: u16,
    pub instance: InstanceId,
    pub invocation_digest: ContentHash,
    pub input_hash: ContentHash,
}

pub struct ArtifactKey<T> {
    pub slot: ArtifactSlot,
    _type: PhantomData<fn() -> T>,
}

impl<T> ArtifactKey<T> {
    pub fn new(slot: ArtifactSlot) -> Self {
        Self {
            slot,
            _type: PhantomData,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArtifactError {
    #[error("missing artifact")]
    Missing,
    #[error("wrong artifact type")]
    WrongType,
    #[error("slot already occupied")]
    Duplicate,
    #[error("artifact provenance does not match slot")]
    ProvenanceMismatch,
}

#[derive(Default)]
pub struct ArtifactStore {
    inner: BTreeMap<ArtifactSlot, Arc<dyn Any + Send + Sync>>,
}

impl ArtifactStore {
    pub fn insert<T: Send + Sync + 'static>(
        &mut self,
        key: ArtifactKey<T>,
        value: Artifact<T>,
    ) -> Result<(), ArtifactError> {
        let p = &value.provenance;
        if p.schema != key.slot.schema
            || p.schema_major != key.slot.major
            || p.analyzer_instance != key.slot.instance
            || p.input_hash != key.slot.input_hash
        {
            return Err(ArtifactError::ProvenanceMismatch);
        }

        if self.inner.contains_key(&key.slot) {
            return Err(ArtifactError::Duplicate);
        }

        self.inner.insert(key.slot, Arc::new(value));
        Ok(())
    }

    pub fn get<T: Send + Sync + 'static>(
        &self,
        key: &ArtifactKey<T>,
    ) -> Result<&Artifact<T>, ArtifactError> {
        self.inner
            .get(&key.slot)
            .ok_or(ArtifactError::Missing)?
            .downcast_ref()
            .ok_or(ArtifactError::WrongType)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandleRequirement {
    pub resolution: Resolution,
    pub min_bars: usize,
    pub desired_bars: usize,
    pub price_unit: PriceUnit,
    pub require_volume: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalysisBinding {
    pub name: BindingId,
    pub instance: InstanceId,
    pub schema: SchemaId,
    pub major: u16,
    pub required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalysisRequirements {
    pub candles: CandleRequirement,
    pub dependencies: Vec<AnalysisBinding>,
}

pub struct AnalysisContext<'a> {
    pub candles: &'a CandleSeries,
    pub dependencies: &'a ArtifactStore,
    pub as_of: TimestampMs,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AnalysisValue {
    Range(analysis_model::RangeAnalysis),
    Volume(analysis_model::VolumeAnalysis),
    Trend(analysis_model::TrendAnalysis),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AnalysisError {
    #[error("invalid analysis input: {0}")]
    InvalidInput(String),
    #[error("insufficient history: {have}/{need}")]
    InsufficientHistory { have: usize, need: usize },
    #[error("unsupported or missing volume units")]
    UnsupportedUnits,
    #[error("missing dependency")]
    MissingDependency,
    #[error("numeric failure")]
    NumericFailure,
}

pub trait Analyzer: Send + Sync {
    /// Stateful analyzers must not share computed outputs across different instances.
    fn stateful(&self) -> bool {
        false
    }

    fn descriptor(&self) -> plugin_api::PluginDescriptor;
    fn instance(&self) -> &InstanceId;
    fn config_digest(&self) -> &ContentHash;
    fn requirements(&self, resolution: Resolution, price_unit: PriceUnit) -> AnalysisRequirements;
    fn analyze(&self, ctx: &AnalysisContext<'_>) -> Result<AnalysisValue, AnalysisError>;
}

#[cfg(test)]
mod tests;
