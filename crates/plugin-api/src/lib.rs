#![forbid(unsafe_code)]
use domain::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    CandidateSource,
    DataProvider,
    Analyzer,
    Strategy,
    NotificationSink,
    StateStore,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub id: CapabilityId,
    pub schema_major: u16,
    pub chains: Vec<ChainId>,
    pub resolutions: Vec<Resolution>,
    pub price_units: Vec<PriceUnit>,
}

impl Capability {
    pub fn pure(id: CapabilityId, schema_major: u16) -> Self {
        Self {
            id,
            schema_major,
            chains: vec![],
            resolutions: vec![],
            price_units: vec![],
        }
    }

    pub fn covers(&self, request: &Capability) -> bool {
        self.id == request.id
            && self.schema_major == request.schema_major
            && request.chains.iter().all(|x| self.chains.contains(x))
            && request
                .resolutions
                .iter()
                .all(|x| self.resolutions.contains(x))
            && request
                .price_units
                .iter()
                .all(|x| self.price_units.contains(x))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub id: PluginId,
    pub version: String,
    pub kind: PluginKind,
    pub provides: Vec<Capability>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Requirement {
    pub binding: BindingId,
    pub provider: InstanceId,
    pub capability: Capability,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfiguredPlugin {
    pub instance: InstanceId,
    pub descriptor: PluginDescriptor,
    pub config_digest: ContentHash,
    pub requires: Vec<Requirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("duplicate plugin {0}")]
    DuplicatePlugin(PluginId),
    #[error("duplicate instance {0}")]
    DuplicateInstance(InstanceId),
    #[error("plugin not compiled: {0}")]
    NotCompiled(PluginId),
    #[error("{consumer} -> {provider}: missing or incompatible capability {capability}")]
    MissingCapability {
        consumer: InstanceId,
        provider: InstanceId,
        capability: CapabilityId,
    },
    #[error("dependency cycle: {0:?}")]
    Cycle(Vec<InstanceId>),
}

#[derive(Default)]
pub struct PluginCatalog {
    entries: BTreeMap<PluginId, PluginDescriptor>,
}

impl PluginCatalog {
    pub fn register(&mut self, descriptor: PluginDescriptor) -> Result<(), RegistryError> {
        if self.entries.contains_key(&descriptor.id) {
            return Err(RegistryError::DuplicatePlugin(descriptor.id));
        }

        self.entries.insert(descriptor.id.clone(), descriptor);
        Ok(())
    }

    pub fn get(&self, id: &PluginId) -> Result<&PluginDescriptor, RegistryError> {
        self.entries
            .get(id)
            .ok_or_else(|| RegistryError::NotCompiled(id.clone()))
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.entries.values()
    }
}

pub fn validate_instances(
    instances: &[ConfiguredPlugin],
) -> Result<Vec<InstanceId>, RegistryError> {
    let mut map = BTreeMap::new();
    for i in instances {
        if map.insert(i.instance.clone(), i).is_some() {
            return Err(RegistryError::DuplicateInstance(i.instance.clone()));
        }
    }

    for i in instances {
        for r in &i.requires {
            if !map.get(&r.provider).is_some_and(|p| {
                p.descriptor
                    .provides
                    .iter()
                    .any(|c| c.covers(&r.capability))
            }) {
                return Err(RegistryError::MissingCapability {
                    consumer: i.instance.clone(),
                    provider: r.provider.clone(),
                    capability: r.capability.id.clone(),
                });
            }
        }
    }

    fn visit(
        id: &InstanceId,
        map: &BTreeMap<InstanceId, &ConfiguredPlugin>,
        path: &mut Vec<InstanceId>,
        done: &mut BTreeSet<InstanceId>,
        out: &mut Vec<InstanceId>,
    ) -> Result<(), RegistryError> {
        if done.contains(id) {
            return Ok(());
        }

        if let Some(start) = path.iter().position(|x| x == id) {
            let mut cycle = path[start..].to_vec();
            cycle.push(id.clone());
            return Err(RegistryError::Cycle(cycle));
        }

        path.push(id.clone());
        for r in &map[id].requires {
            visit(&r.provider, map, path, done, out)?;
        }

        path.pop();
        done.insert(id.clone());
        out.push(id.clone());
        Ok(())
    }

    let mut out = vec![];
    let mut done = BTreeSet::new();
    for id in map.keys() {
        visit(id, &map, &mut vec![], &mut done, &mut out)?;
    }

    Ok(out)
}

#[cfg(test)]
mod tests;
