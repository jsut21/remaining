use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderId {
    Zai,
    Codex,
    OpenCode,
    Ollama,
}

impl ProviderId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zai => "zai",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Ollama => "ollama",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStatus {
    Available,
    AuthRequired,
    NetworkError,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowType {
    FiveHours,
    Weekly,
    Monthly,
    Other,
}

#[derive(Debug, Clone)]
pub struct QuotaWindow {
    pub wtype: WindowType,
    pub label: String,
    pub used_percent: i32,
    pub limit: i64,
    pub remaining: i64,
    pub resets_at_epoch: Option<i64>,
    pub resets_at_str: Option<String>,
    #[allow(dead_code)]
    pub has_real_remaining: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderResult {
    pub provider: ProviderId,
    pub name: String,
    pub status: ProviderStatus,
    pub error: Option<String>,
    pub plan: Option<String>,
    pub windows: Vec<QuotaWindow>,
    pub fetched_at: chrono::DateTime<chrono::Local>,
}

impl ProviderResult {
    #[allow(dead_code)]
    pub fn auth_required(id: ProviderId) -> Self {
        Self {
            provider: id,
            name: id.as_str().to_string(),
            status: ProviderStatus::AuthRequired,
            error: Some("No credentials found".into()),
            plan: None,
            windows: vec![],
            fetched_at: chrono::Local::now(),
        }
    }

    pub fn network_error(id: ProviderId, msg: impl Into<String>) -> Self {
        Self {
            provider: id,
            name: id.as_str().to_string(),
            status: ProviderStatus::NetworkError,
            error: Some(msg.into()),
            plan: None,
            windows: vec![],
            fetched_at: chrono::Local::now(),
        }
    }

    pub fn unavailable(id: ProviderId, msg: impl Into<String>) -> Self {
        Self {
            provider: id,
            name: id.as_str().to_string(),
            status: ProviderStatus::Unavailable,
            error: Some(msg.into()),
            plan: None,
            windows: vec![],
            fetched_at: chrono::Local::now(),
        }
    }
}