//! Transport-independent agent-to-PTY binding state.
//!
//! A PTY id alone is not enough to authorize a binding: ids can be reused
//! after daemon restarts and connections can observe ids owned by another
//! client. Every operation therefore verifies the PTY generation and its
//! currently attached connection before changing agent state.

use std::collections::HashMap;

/// Stable identity of a CLI-agent conversation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AgentIdentity {
    pub provider: String,
    pub session_id: String,
    pub account_email: Option<String>,
    pub config_dir: Option<String>,
}

/// A request to make an agent the live foreground agent for a PTY.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingRequest {
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
    ForeignConnection,
    ForegroundConflict,
    HandoffMismatch,
    IdentityNotBound,
    IdentityAlreadyBound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PtyRegistration {
    generation: u64,
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
    /// Reusing an id for a new generation drops all bindings for the old
    /// generation so stale agent inventory can never attach to the new PTY.
    pub fn register_pty(
        &mut self,
        pty_session_id: impl Into<String>,
        generation: u64,
        attached_connection: u128,
    ) {
        let pty_session_id = pty_session_id.into();
        if self
            .ptys
            .get(&pty_session_id)
            .is_some_and(|registration| registration.generation != generation)
        {
            self.bindings
                .retain(|binding| binding.pty_session_id != pty_session_id);
        }
        self.ptys.insert(
            pty_session_id,
            PtyRegistration {
                generation,
                attached_connection,
            },
        );
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

    /// Removes a PTY generation and every current or historical binding to it.
    pub fn remove_pty(&mut self, pty_session_id: &str, generation: u64) {
        let should_remove = self
            .ptys
            .get(pty_session_id)
            .is_some_and(|registration| registration.generation == generation);
        if !should_remove {
            return;
        }
        self.ptys.remove(pty_session_id);
        self.bindings.retain(|binding| {
            binding.pty_session_id != pty_session_id || binding.pty_generation != generation
        });
    }

    /// Makes an agent the only foreground agent for a validated PTY.
    pub fn bind(&mut self, connection: u128, request: BindingRequest) -> Result<(), BindingError> {
        self.validate_pty(connection, &request.pty_session_id, request.pty_generation)?;
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
        } else if request.handoff_from.is_some() {
            return Err(BindingError::HandoffMismatch);
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
        Ok(())
    }

    /// Marks one identity historical after validating PTY ownership.
    ///
    /// The association remains available to inventory, but it is no longer
    /// attachable. Removing the PTY generation removes its history entirely.
    pub fn unbind(
        &mut self,
        connection: u128,
        agent: &AgentIdentity,
        pty_session_id: &str,
        generation: u64,
    ) -> Result<(), BindingError> {
        self.validate_pty(connection, pty_session_id, generation)?;
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

    fn validate_pty(
        &self,
        connection: u128,
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
        if registration.attached_connection != connection {
            return Err(BindingError::ForeignConnection);
        }
        Ok(())
    }
}
