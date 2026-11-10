use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub instance_id: uuid::Uuid,
    pub environment: Environment,
    pub location_id: String,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Production,
    Sandbox,
}

impl Environment {
    pub fn origin(self) -> &'static str {
        match self {
            Self::Production => "https://api.usestable.com",
            Self::Sandbox => "https://api.dev.usestable.com",
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Sandbox => "sandbox",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MailItem {
    pub id: String,
    pub sender: String,
    pub received_at: Option<String>,
    pub first_seen_at: i64,
    pub observed_at: i64,
    pub archived_at: Option<String>,
    pub read_at: Option<String>,
    pub clear_at: Option<String>,
    pub scan_status: String,
    pub scan_notice: Option<String>,
    pub shred_status: Option<String>,
    pub summary: Option<String>,
    pub has_envelope: bool,
    pub has_pdf: bool,
    pub has_ocr: bool,
    pub revision: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub query: Option<String>,
    pub status: Option<String>,
    pub archived: Option<bool>,
    pub unread: Option<bool>,
    pub limit: Option<u32>,
    pub page_token: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub total: i64,
    pub unread: i64,
    pub pending_scans: i64,
    pub ready_scans: i64,
    pub storage_due: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncStatus {
    pub last_success_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    pub error_code: Option<String>,
    pub initial_sync_complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncOperation {
    pub id: uuid::Uuid,
    pub state: OperationState,
    pub error_code: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Queued,
    Running,
    Completed,
    Failed,
}

impl TryFrom<&str> for OperationState {
    type Error = crate::error::Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(crate::error::Error::Corrupt),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub sequence: i64,
    pub item_id: String,
    pub kind: EventKind,
    pub observed_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    NewMail,
    ScanReady,
    ScanNotice,
    StorageDue,
}

impl TryFrom<&str> for EventKind {
    type Error = crate::error::Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "new_mail" => Ok(Self::NewMail),
            "scan_ready" => Ok(Self::ScanReady),
            "scan_notice" => Ok(Self::ScanNotice),
            "storage_due" => Ok(Self::StorageDue),
            _ => Err(crate::error::Error::Corrupt),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventHead {
    pub sequence: i64,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ocr {
    pub pages: Vec<String>,
}

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}
