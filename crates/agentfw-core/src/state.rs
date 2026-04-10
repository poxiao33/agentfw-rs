use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AudienceState {
    pub visible_to: Vec<String>,
}

impl AudienceState {
    /// Normalizes a list of recipients by sorting and deduplicating.
    pub fn normalize(mut visible_to: Vec<String>) -> Self {
        visible_to.sort();
        visible_to.dedup();
        Self { visible_to }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudienceOnMissing {
    Error,
    UseState { state: AudienceState },
}

impl Default for AudienceOnMissing {
    fn default() -> Self {
        // Default to an empty audience rather than an error so that dispatch_content
        // does not hard-fail when audience state has not been explicitly initialized.
        // Agents with no audience simply produce no delivered messages.
        Self::UseState {
            state: AudienceState {
                visible_to: Vec::new(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEffect {
    SetAudience {
        visible_to: Vec<String>,
    },
    AppendHistory {
        messages: Vec<crate::message::Message>,
    },
    ArchivePayload {
        reference: String,
        payload: String,
    },
    Custom {
        name: String,
        payload: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    pub session_id: crate::message::SessionId,
    pub metadata: Value,
}
