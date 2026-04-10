use crate::error::FrameworkError;
use crate::message::Message;
use crate::state::{AudienceOnMissing, AudienceState};
use std::collections::HashMap;

pub trait HistoryStore {
    fn load(&self, session_id: &str, agent_id: &str) -> Result<Vec<Message>, FrameworkError>;
    fn append(
        &mut self,
        session_id: &str,
        agent_id: &str,
        msgs: Vec<Message>,
    ) -> Result<(), FrameworkError>;
    fn replace(
        &mut self,
        session_id: &str,
        agent_id: &str,
        msgs: Vec<Message>,
    ) -> Result<(), FrameworkError>;
}

pub trait ArchiveStore {
    fn save(&mut self, reference: &str, payload: &str) -> Result<(), FrameworkError>;
    fn load(&self, reference: &str) -> Result<Option<String>, FrameworkError>;
}

pub trait AudienceStateStore {
    fn get(&self, session_id: &str, agent_id: &str) -> Result<AudienceState, FrameworkError>;
    fn set(
        &mut self,
        session_id: &str,
        agent_id: &str,
        state: AudienceState,
    ) -> Result<(), FrameworkError>;

    fn set_on_missing_policy(&mut self, policy: AudienceOnMissing) -> Result<(), FrameworkError>;
}

#[derive(Default)]
pub(crate) struct InMemoryHistoryStore {
    data: HashMap<String, HashMap<String, Vec<Message>>>,
}

impl InMemoryHistoryStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

impl HistoryStore for InMemoryHistoryStore {
    fn load(&self, session_id: &str, agent_id: &str) -> Result<Vec<Message>, FrameworkError> {
        Ok(self
            .data
            .get(session_id)
            .and_then(|m| m.get(agent_id))
            .cloned()
            .unwrap_or_default())
    }

    fn append(
        &mut self,
        session_id: &str,
        agent_id: &str,
        msgs: Vec<Message>,
    ) -> Result<(), FrameworkError> {
        self.data
            .entry(session_id.to_string())
            .or_default()
            .entry(agent_id.to_string())
            .or_default()
            .extend(msgs);
        Ok(())
    }

    fn replace(
        &mut self,
        session_id: &str,
        agent_id: &str,
        msgs: Vec<Message>,
    ) -> Result<(), FrameworkError> {
        self.data
            .entry(session_id.to_string())
            .or_default()
            .insert(agent_id.to_string(), msgs);
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct InMemoryArchiveStore {
    data: HashMap<String, String>,
}

impl InMemoryArchiveStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

impl ArchiveStore for InMemoryArchiveStore {
    fn save(&mut self, reference: &str, payload: &str) -> Result<(), FrameworkError> {
        if self.data.contains_key(reference) {
            return Err(FrameworkError::Store(format!(
                "archive reference already exists: '{}'; use a unique reference or remove the existing entry",
                reference
            )));
        }
        self.data.insert(reference.to_string(), payload.to_string());
        Ok(())
    }

    fn load(&self, reference: &str) -> Result<Option<String>, FrameworkError> {
        Ok(self.data.get(reference).cloned())
    }
}

#[derive(Default)]
pub(crate) struct InMemoryAudienceStateStore {
    data: HashMap<String, HashMap<String, AudienceState>>,
    on_missing: AudienceOnMissing,
}

impl InMemoryAudienceStateStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

impl AudienceStateStore for InMemoryAudienceStateStore {
    fn get(&self, session_id: &str, agent_id: &str) -> Result<AudienceState, FrameworkError> {
        if let Some(state) = self
            .data
            .get(session_id)
            .and_then(|m| m.get(agent_id))
            .cloned()
        {
            return Ok(state);
        }

        match &self.on_missing {
            AudienceOnMissing::Error => Err(FrameworkError::Store(format!(
                "audience state not initialized for session={} agent={}",
                session_id, agent_id
            ))),
            AudienceOnMissing::UseState { state } => Ok(state.clone()),
        }
    }

    fn set(
        &mut self,
        session_id: &str,
        agent_id: &str,
        state: AudienceState,
    ) -> Result<(), FrameworkError> {
        self.data
            .entry(session_id.to_string())
            .or_default()
            .insert(agent_id.to_string(), state);
        Ok(())
    }

    fn set_on_missing_policy(&mut self, policy: AudienceOnMissing) -> Result<(), FrameworkError> {
        self.on_missing = policy;
        Ok(())
    }
}
