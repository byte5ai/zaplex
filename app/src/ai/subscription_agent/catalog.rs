use super::{AgentCapability, InstallationIdentity, ModelCapability};
use std::collections::HashMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CapabilityRefresh {
    pub(crate) selected_model: Option<ModelCapability>,
    pub(crate) selection_invalidated: bool,
}

/// Version-keyed capabilities reported by installed official CLI processes.
#[derive(Default)]
pub(crate) struct CapabilityCatalog {
    capabilities: HashMap<InstallationIdentity, AgentCapability>,
}

impl CapabilityCatalog {
    pub(crate) fn get(&self, installation: &InstallationIdentity) -> Option<&AgentCapability> {
        self.capabilities.get(installation)
    }

    pub(crate) fn all(&self) -> impl Iterator<Item = &AgentCapability> {
        self.capabilities.values()
    }

    /// Atomically replaces one installation's reported capabilities.
    ///
    /// A selection is retained only when its exact model ID is still present. The returned
    /// model always comes from the new report so associated metadata cannot become stale.
    pub(crate) fn replace(
        &mut self,
        capability: AgentCapability,
        selected_model_id: Option<&str>,
    ) -> CapabilityRefresh {
        let selected_model = selected_model_id.and_then(|selected_model_id| {
            capability
                .models
                .iter()
                .find(|model| model.id == selected_model_id)
                .cloned()
        });
        let selection_invalidated = selected_model_id.is_some() && selected_model.is_none();
        self.capabilities
            .insert(capability.installation.clone(), capability);
        CapabilityRefresh {
            selected_model,
            selection_invalidated,
        }
    }

    pub(crate) fn remove(&mut self, installation: &InstallationIdentity) {
        self.capabilities.remove(installation);
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
