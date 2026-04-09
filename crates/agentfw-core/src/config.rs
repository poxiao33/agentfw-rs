use crate::agent::AgentSpec;
use crate::error::FrameworkError;
use crate::message::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouteRule {
    pub from: String,
    pub to: String,
    pub allow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionSpec {
    pub id: String,
    #[serde(default)]
    pub agents: Vec<AgentSpec>,
    #[serde(default)]
    pub routes: Vec<RouteRule>,
    #[serde(default)]
    pub metadata: Value,
}

impl SessionSpec {
    pub fn normalized_routes(&self) -> Result<Vec<RouteRule>, FrameworkError> {
        let mut seen: HashMap<(String, String), bool> = HashMap::new();
        for rule in &self.routes {
            let from = rule.from.trim();
            let to = rule.to.trim();
            if from.is_empty() || to.is_empty() {
                return Err(FrameworkError::Config(
                    "route rule requires non-empty from/to".to_string(),
                ));
            }
            let key = (from.to_string(), to.to_string());
            if let Some(existing) = seen.get(&key) {
                if *existing != rule.allow {
                    return Err(FrameworkError::Config(format!(
                        "conflicting route rules for {} -> {}",
                        from, to
                    )));
                }
            } else {
                seen.insert(key, rule.allow);
            }
        }
        Ok(seen
            .into_iter()
            .map(|((from, to), allow)| RouteRule { from, to, allow })
            .collect())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StaticPromptMap {
    #[serde(default)]
    pub prompts: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StaticToolBinding {
    pub agent_id: String,
    #[serde(default)]
    pub tool_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StaticHistoryBinding {
    #[serde(default)]
    pub session_id: String,
    pub agent_id: String,
    #[serde(default)]
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StaticModelBinding {
    pub key: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key_env: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StaticConfig {
    pub session: SessionSpec,
    #[serde(default)]
    pub prompts: HashMap<String, String>,
    #[serde(default)]
    pub models: Vec<StaticModelBinding>,
    #[serde(default)]
    pub tool_bindings: Vec<StaticToolBinding>,
    #[serde(default)]
    pub history_bindings: Vec<StaticHistoryBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeveloperConfig {
    pub session: SessionSpec,
    #[serde(default)]
    pub prompts: HashMap<String, String>,
    #[serde(default)]
    pub models: Vec<StaticModelBinding>,
    #[serde(default)]
    pub bindings: DeveloperBindings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeveloperBindings {
    #[serde(default)]
    pub tools: Vec<StaticToolBinding>,
    #[serde(default)]
    pub history: Vec<StaticHistoryBinding>,
}

impl StaticConfig {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, FrameworkError> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|err| {
            FrameworkError::Store(format!("failed to read config {}: {err}", path.display()))
        })?;

        match path.extension().and_then(|ext| ext.to_str()) {
            Some("json") => Self::from_json_str(&content),
            Some("toml") => Self::from_toml_str(&content),
            Some(other) => Err(FrameworkError::Config(format!(
                "unsupported config extension: {other}"
            ))),
            None => Err(FrameworkError::Config(
                "config path must have .json or .toml extension".to_string(),
            )),
        }
    }

    pub fn from_json_str(input: &str) -> Result<Self, FrameworkError> {
        serde_json::from_str(input)
            .map_err(|err| FrameworkError::Config(format!("invalid static config json: {err}")))
    }

    pub fn from_toml_str(input: &str) -> Result<Self, FrameworkError> {
        toml::from_str(input)
            .map_err(|err| FrameworkError::Config(format!("invalid static config toml: {err}")))
    }

    pub fn validate(&self) -> Result<(), FrameworkError> {
        if self.session.id.trim().is_empty() {
            return Err(FrameworkError::Config(
                "static config requires non-empty session.id".to_string(),
            ));
        }
        // Use trimmed IDs as keys so lookup is consistent with how routes are normalized.
        let mut known_agents: HashMap<String, ()> = HashMap::new();
        for agent in &self.session.agents {
            let id = agent.id.trim();
            if id.is_empty() {
                return Err(FrameworkError::Config(
                    "static config requires non-empty agent.id".to_string(),
                ));
            }
            if agent.driver.trim().is_empty() {
                return Err(FrameworkError::Config(format!(
                    "agent {} requires non-empty driver",
                    id
                )));
            }
            if agent.prompt_ref.trim().is_empty() {
                return Err(FrameworkError::Config(format!(
                    "agent {} requires non-empty prompt_ref",
                    id
                )));
            }
            if agent.model_ref.trim().is_empty() {
                return Err(FrameworkError::Config(format!(
                    "agent {} requires non-empty model_ref",
                    id
                )));
            }
            if known_agents.insert(id.to_string(), ()).is_some() {
                return Err(FrameworkError::Config(format!(
                    "duplicate agent id in static config: {}",
                    id
                )));
            }
            if !self.prompts.contains_key(agent.prompt_ref.trim()) {
                return Err(FrameworkError::Config(format!(
                    "prompt ref not found for agent {}: {}",
                    id, agent.prompt_ref.trim()
                )));
            }
        }

        let mut known_models = HashMap::new();
        for model in &self.models {
            if model.key.trim().is_empty() {
                return Err(FrameworkError::Config(
                    "static model binding requires non-empty key".to_string(),
                ));
            }
            if model.provider.trim().is_empty() {
                return Err(FrameworkError::Config(format!(
                    "static model binding {} requires non-empty provider",
                    model.key
                )));
            }
            if model.model.trim().is_empty() {
                return Err(FrameworkError::Config(format!(
                    "static model binding {} requires non-empty model",
                    model.key
                )));
            }
            if known_models.insert(model.key.clone(), ()).is_some() {
                return Err(FrameworkError::Config(format!(
                    "duplicate model key in static config: {}",
                    model.key
                )));
            }
        }

        for agent in &self.session.agents {
            if !known_models.contains_key(agent.model_ref.trim()) {
                return Err(FrameworkError::Config(format!(
                    "model ref not found for agent {}: {}",
                    agent.id.trim(), agent.model_ref.trim()
                )));
            }
        }

        for route in self.session.normalized_routes()?.iter() {
            if !known_agents.contains_key(&route.from) {
                return Err(FrameworkError::Config(format!(
                    "route.from references unknown agent: {}",
                    route.from
                )));
            }
            if !known_agents.contains_key(&route.to) {
                return Err(FrameworkError::Config(format!(
                    "route.to references unknown agent: {}",
                    route.to
                )));
            }
        }

        for binding in &self.tool_bindings {
            if !known_agents.contains_key(&binding.agent_id) {
                return Err(FrameworkError::Config(format!(
                    "tool binding references unknown agent: {}",
                    binding.agent_id
                )));
            }
        }

        for binding in &self.history_bindings {
            if !known_agents.contains_key(&binding.agent_id) {
                return Err(FrameworkError::Config(format!(
                    "history binding references unknown agent: {}",
                    binding.agent_id
                )));
            }
        }
        Ok(())
    }
}

impl DeveloperConfig {
    pub fn into_static(self) -> StaticConfig {
        StaticConfig {
            session: self.session,
            prompts: self.prompts,
            models: self.models,
            tool_bindings: self.bindings.tools,
            history_bindings: self.bindings.history,
        }
    }

    fn to_static(&self) -> StaticConfig {
        StaticConfig {
            session: self.session.clone(),
            prompts: self.prompts.clone(),
            models: self.models.clone(),
            tool_bindings: self.bindings.tools.clone(),
            history_bindings: self.bindings.history.clone(),
        }
    }

    pub fn from_json_str(input: &str) -> Result<Self, FrameworkError> {
        serde_json::from_str(input)
            .map_err(|err| FrameworkError::Config(format!("invalid developer config json: {err}")))
    }

    pub fn from_toml_str(input: &str) -> Result<Self, FrameworkError> {
        toml::from_str(input)
            .map_err(|err| FrameworkError::Config(format!("invalid developer config toml: {err}")))
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, FrameworkError> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|err| {
            FrameworkError::Store(format!("failed to read config {}: {err}", path.display()))
        })?;

        match path.extension().and_then(|ext| ext.to_str()) {
            Some("json") => Self::from_json_str(&content),
            Some("toml") => Self::from_toml_str(&content),
            Some(other) => Err(FrameworkError::Config(format!(
                "unsupported config extension: {other}"
            ))),
            None => Err(FrameworkError::Config(
                "config path must have .json or .toml extension".to_string(),
            )),
        }
    }

    pub fn validate(&self) -> Result<(), FrameworkError> {
        self.to_static().validate()
    }
}
