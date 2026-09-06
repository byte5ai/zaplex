//! Transport-independent agent-to-PTY binding state.
//!
//! A PTY id alone is not enough to authorize a binding: ids can be reused
//! after daemon restarts and connections can observe ids owned by another
//! client. Every operation therefore verifies the daemon identity, PTY
//! generation, and currently attached connection before changing agent state.

use std::collections::{HashMap, HashSet};

/// Maximum number of closed, non-foreground binding records retained by the daemon.
pub const MAX_HISTORICAL_BINDINGS: usize = 256;

/// Stable identity of a CLI-agent conversation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AgentIdentity {
    pub provider: String,
    pub session_id: String,
    pub account_email: Option<String>,
    pub config_dir: Option<String>,
    pub account_id: Option<String>,
}

/// A request to make an agent the live foreground agent for a PTY.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingRequest {
    pub host_id: String,
    pub pty_session_id: String,
    pub pty_generation: u64,
    pub agent: AgentIdentity,
    pub handoff_from: Option<AgentIdentity>,
}

/// One current or historical agent association with a PTY generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentPtyBinding {
    pub agent: AgentIdentity,
    pub pty_session_id: String,
    pub pty_generation: u64,
    pub foreground: bool,
}

/// A validation failure that leaves the binding registry unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingError {
    PtyNotFound,
    StaleGeneration,
    ForeignDaemon,
    ForeignConnection,
    ForegroundConflict,
    HandoffMismatch,
    IdentityNotBound,
    IdentityAlreadyBound,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PtyRegistration {
    generation: u64,
    host_id: String,
    attached_connection: u128,
}

/// In-memory binding registry owned by the daemon.
#[derive(Default)]
pub struct AgentPtyBindings {
    ptys: HashMap<String, PtyRegistration>,
    bindings: Vec<AgentPtyBinding>,
}

impl AgentPtyBindings {
    /// Registers a newly opened PTY generation.
    ///
    /// Historical bindings for an older generation remain non-attachable and
    /// generation-qualified when an id is reused, subject to the closed-history cap.
    pub fn register_pty(
        &mut self,
        pty_session_id: impl Into<String>,
        generation: u64,
        host_id: impl Into<String>,
        attached_connection: u128,
    ) {
        let pty_session_id = pty_session_id.into();
        let host_id = host_id.into();
        if self.ptys.get(&pty_session_id).is_some_and(|registration| {
            registration.generation != generation || registration.host_id != host_id
        }) {
            self.close_registered_generation(&pty_session_id);
        }
        self.ptys.insert(
            pty_session_id,
            PtyRegistration {
                generation,
                host_id,
                attached_connection,
            },
        );
        self.prune_closed_history();
    }

    /// Transfers an existing PTY generation to a reconnected client.
    pub fn attach_pty(
        &mut self,
        pty_session_id: &str,
        generation: u64,
        attached_connection: u128,
    ) -> Result<(), BindingError> {
        let registration = self
            .ptys
            .get_mut(pty_session_id)
            .ok_or(BindingError::PtyNotFound)?;
        if registration.generation != generation {
            return Err(BindingError::StaleGeneration);
        }
        registration.attached_connection = attached_connection;
        Ok(())
    }

    /// Closes a PTY generation and retains its newest historical associations up to
    /// [`MAX_HISTORICAL_BINDINGS`]. Bindings for registered generations are never pruned.
    pub fn remove_pty(&mut self, pty_session_id: &str, generation: u64) {
        let should_remove = self
            .ptys
            .get(pty_session_id)
            .is_some_and(|registration| registration.generation == generation);
        if !should_remove {
            return;
        }
        self.ptys.remove(pty_session_id);
        for binding in &mut self.bindings {
            if binding.pty_session_id == pty_session_id && binding.pty_generation == generation {
                binding.foreground = false;
            }
        }
        self.prune_closed_history();
    }

    /// Makes an agent the only foreground agent for a validated PTY.
    pub fn bind(&mut self, connection: u128, request: BindingRequest) -> Result<(), BindingError> {
        self.validate_pty(
            connection,
            &request.host_id,
            &request.pty_session_id,
            request.pty_generation,
        )?;
        if self.bindings.iter().any(|binding| {
            binding.agent == request.agent
                && binding.foreground
                && (binding.pty_session_id != request.pty_session_id
                    || binding.pty_generation != request.pty_generation)
        }) {
            return Err(BindingError::IdentityAlreadyBound);
        }

        let foreground = self.bindings.iter().position(|binding| {
            binding.pty_session_id == request.pty_session_id
                && binding.pty_generation == request.pty_generation
                && binding.foreground
        });

        if let Some(foreground) = foreground {
            let foreground_agent = &self.bindings[foreground].agent;
            if foreground_agent == &request.agent {
                self.prune_closed_history();
                return Ok(());
            }
            match request.handoff_from.as_ref() {
                None => return Err(BindingError::ForegroundConflict),
                Some(handoff_from) if handoff_from != foreground_agent => {
                    return Err(BindingError::HandoffMismatch);
                }
                Some(_) => {
                    self.bindings[foreground].foreground = false;
                }
            }
        } else if let Some(handoff_from) = request.handoff_from.as_ref() {
            let is_known_history = self.bindings.iter().any(|binding| {
                binding.agent == *handoff_from
                    && binding.pty_session_id == request.pty_session_id
                    && binding.pty_generation == request.pty_generation
                    && !binding.foreground
            });
            if !is_known_history {
                return Err(BindingError::HandoffMismatch);
            }
        }

        if let Some(existing) = self.bindings.iter_mut().find(|binding| {
            binding.agent == request.agent
                && binding.pty_session_id == request.pty_session_id
                && binding.pty_generation == request.pty_generation
        }) {
            existing.foreground = true;
        } else {
            self.bindings.push(AgentPtyBinding {
                agent: request.agent,
                pty_session_id: request.pty_session_id,
                pty_generation: request.pty_generation,
                foreground: true,
            });
        }
        self.prune_closed_history();
        Ok(())
    }

    /// Marks one identity historical after validating PTY ownership.
    ///
    /// The association remains available while its PTY generation is registered.
    /// After that generation closes, it becomes subject to the bounded history policy.
    pub fn unbind(
        &mut self,
        connection: u128,
        host_id: &str,
        agent: &AgentIdentity,
        pty_session_id: &str,
        generation: u64,
    ) -> Result<(), BindingError> {
        self.validate_pty(connection, host_id, pty_session_id, generation)?;
        let binding = self
            .bindings
            .iter_mut()
            .find(|binding| {
                binding.agent == *agent
                    && binding.pty_session_id == pty_session_id
                    && binding.pty_generation == generation
            })
            .ok_or(BindingError::IdentityNotBound)?;
        binding.foreground = false;
        self.prune_closed_history();
        Ok(())
    }

    pub fn binding_for(&self, agent: &AgentIdentity) -> Option<&AgentPtyBinding> {
        self.bindings
            .iter()
            .find(|binding| binding.agent == *agent && binding.foreground)
            .or_else(|| {
                self.bindings
                    .iter()
                    .rev()
                    .find(|binding| binding.agent == *agent)
            })
    }

    pub fn bindings_for_pty(&self, pty_session_id: &str, generation: u64) -> Vec<&AgentPtyBinding> {
        self.bindings
            .iter()
            .filter(|binding| {
                binding.pty_session_id == pty_session_id && binding.pty_generation == generation
            })
            .collect()
    }

    pub fn foreground_for_pty(
        &self,
        pty_session_id: &str,
        generation: u64,
    ) -> Option<&AgentPtyBinding> {
        self.bindings.iter().find(|binding| {
            binding.pty_session_id == pty_session_id
                && binding.pty_generation == generation
                && binding.foreground
        })
    }

    /// Makes bindings whose agents disappeared from the current live inventory
    /// historical, then prunes the oldest closed-generation history above the cap.
    pub fn reconcile_live_agents(&mut self, live_agents: &HashSet<AgentIdentity>) {
        for binding in &mut self.bindings {
            if binding.foreground && !live_agents.contains(&binding.agent) {
                binding.foreground = false;
            }
        }
        self.prune_closed_history();
    }

    fn close_registered_generation(&mut self, pty_session_id: &str) {
        let Some(registration) = self.ptys.get(pty_session_id) else {
            return;
        };
        let generation = registration.generation;
        for binding in &mut self.bindings {
            if binding.pty_session_id == pty_session_id && binding.pty_generation == generation {
                binding.foreground = false;
            }
        }
    }

    fn prune_closed_history(&mut self) {
        let ptys = &self.ptys;
        let is_closed_history = |binding: &AgentPtyBinding| {
            !binding.foreground
                && !ptys
                    .get(&binding.pty_session_id)
                    .is_some_and(|registration| registration.generation == binding.pty_generation)
        };
        let mut to_prune = self
            .bindings
            .iter()
            .filter(|binding| is_closed_history(binding))
            .count()
            .saturating_sub(MAX_HISTORICAL_BINDINGS);

        self.bindings.retain(|binding| {
            if to_prune > 0 && is_closed_history(binding) {
                to_prune -= 1;
                false
            } else {
                true
            }
        });
    }

    fn validate_pty(
        &self,
        connection: u128,
        host_id: &str,
        pty_session_id: &str,
        generation: u64,
    ) -> Result<(), BindingError> {
        let registration = self
            .ptys
            .get(pty_session_id)
            .ok_or(BindingError::PtyNotFound)?;
        if registration.generation != generation {
            return Err(BindingError::StaleGeneration);
        }
        if registration.host_id != host_id {
            return Err(BindingError::ForeignDaemon);
        }
        if registration.attached_connection != connection {
            return Err(BindingError::ForeignConnection);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "agent_binding_tests.rs"]
mod tests;
