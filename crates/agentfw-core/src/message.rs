use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static MSG_COUNTER: AtomicU64 = AtomicU64::new(0);

macro_rules! impl_id_conversions {
    ($t:ty) => {
        impl From<&str> for $t {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }
        impl From<String> for $t {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Standard,
    System,
    Tool,
}

impl Default for MessageKind {
    fn default() -> Self {
        Self::Standard
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct AgentId(pub String);

impl AgentId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl_id_conversions!(AgentId);

impl core::fmt::Display for AgentId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct MessageId(pub String);

impl MessageId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl_id_conversions!(MessageId);

impl core::fmt::Display for MessageId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl_id_conversions!(SessionId);
impl_id_conversions!(Timestamp);

impl core::fmt::Display for SessionId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(transparent)]
pub struct Timestamp(pub String);

impl Timestamp {
    pub fn now_utc_rfc3339() -> Self {
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(std::time::Duration::from_secs(0))
            .as_secs();
        Self(unix_secs_to_rfc3339(unix))
    }
}

/// Convert Unix seconds to an RFC3339/ISO-8601 UTC string.
/// Uses the proleptic Gregorian calendar algorithm (handles leap years correctly).
fn unix_secs_to_rfc3339(unix: u64) -> String {
    let s = (unix % 60) as u32;
    let m = ((unix / 60) % 60) as u32;
    let h = ((unix / 3600) % 24) as u32;

    // Days since 1970-01-01
    let mut days = (unix / 86400) as u32;

    // Shift epoch to 1 Mar 0000 to simplify leap-year math
    days += 719468;
    let era = days / 146097;
    let doe = days % 146097; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month of year [0, 11] (Mar=0)
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let mo = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let yr = if mo <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", yr, mo, d, h, m, s)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: MessageId,
    pub session_id: SessionId,
    #[serde(default)]
    pub kind: MessageKind,
    pub from: AgentId,
    pub to: AgentId,
    pub content: Vec<ContentBlock>,
    pub meta: MessageMeta,
    pub created_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageDraft {
    #[serde(default)]
    pub kind: MessageKind,
    pub from: AgentId,
    pub to: AgentId,
    pub content: Vec<ContentBlock>,
    pub meta: MessageMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MessageMeta {
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolCall {
        tool_name: String,
        arguments: Value,
        call_id: Option<String>,
    },
    ToolResult {
        tool_name: String,
        content: Value,
        call_id: String,
        status: ToolResultStatus,
    },
    Image {
        url: String,
    },
    Reference {
        reference: String,
    },
    System {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    Success,
    Error,
    Partial,
    Cancelled,
}

impl Default for ToolResultStatus {
    fn default() -> Self {
        Self::Success
    }
}

impl Message {
    pub fn text(
        from: impl Into<AgentId>,
        to: impl Into<AgentId>,
        text: impl Into<String>,
    ) -> MessageDraft {
        MessageDraft {
            kind: MessageKind::Standard,
            from: from.into(),
            to: to.into(),
            content: vec![ContentBlock::Text { text: text.into() }],
            meta: MessageMeta::default(),
        }
    }
}

impl MessageDraft {
    pub fn commit(self, session_id: impl Into<SessionId>, id: impl Into<MessageId>) -> Message {
        let mut id = id.into();
        if id.0.trim().is_empty() {
            let unix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(std::time::Duration::from_secs(0))
                .as_nanos();
            let seq = MSG_COUNTER.fetch_add(1, Ordering::Relaxed);
            id = MessageId(format!("msg:{unix}-{seq}"));
        }
        Message {
            id,
            session_id: session_id.into(),
            kind: self.kind,
            from: self.from,
            to: self.to,
            content: self.content,
            meta: self.meta,
            created_at: Some(Timestamp::now_utc_rfc3339()),
        }
    }

    pub fn commit_auto(self, session_id: impl Into<SessionId>) -> Message {
        self.commit(session_id, MessageId::default())
    }
}
