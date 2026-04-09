use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSpec {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub prompt_ref: String,
    pub model_ref: String,
    #[serde(default)]
    pub metadata: Value,
}

impl AgentSpec {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        driver: impl Into<String>,
        prompt_ref: impl Into<String>,
        model_ref: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            driver: driver.into(),
            prompt_ref: prompt_ref.into(),
            model_ref: model_ref.into(),
            metadata: Value::Null,
        }
    }
}
