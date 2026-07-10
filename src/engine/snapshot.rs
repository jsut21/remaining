use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderId {
    Zai,
    Codex,
    OpenCode,
    Ollama,
    MiniMax,
}

impl ProviderId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zai => "zai",
            Self::Codex => "codex",
            Self::OpenCode => "opencode-go",
            Self::Ollama => "ollama",
            Self::MiniMax => "minimax",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Zai => "Z.ai",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::Ollama => "Ollama",
            Self::MiniMax => "MiniMax",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowType {
    FiveHours,
    Weekly,
    Monthly,
    Other,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchSource {
    OAuthApi,
    ApiKey,
    BrowserCookie,
    WebScrape,
    LocalFile,
    LocalDatabase,
    Cli,
    Unknown,
}

impl FetchSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OAuthApi => "oauth_api",
            Self::ApiKey => "api_key",
            Self::BrowserCookie => "browser_cookie",
            Self::WebScrape => "web_scrape",
            Self::LocalFile => "local_file",
            Self::LocalDatabase => "local_database",
            Self::Cli => "cli",
            Self::Unknown => "unknown",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaUnit {
    Percent,
    Tokens,
    Credits,
    Requests,
    Minutes,
    Usd,
    Units,
}

impl QuotaUnit {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Percent => "percent",
            Self::Tokens => "tokens",
            Self::Credits => "credits",
            Self::Requests => "requests",
            Self::Minutes => "minutes",
            Self::Usd => "usd",
            Self::Units => "units",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreditBalance {
    pub label: String,
    pub remaining: Option<i64>,
    pub total: Option<i64>,
    pub unit: QuotaUnit,
    pub expires_at_epoch: Option<i64>,
    pub source_label: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UsageMetric {
    pub label: String,
    pub value: f64,
    pub unit: QuotaUnit,
    pub period: Option<String>,
    pub source_label: Option<String>,
}

#[derive(Debug, Clone)]
pub struct QuotaWindow {
    pub wtype: WindowType,
    pub label: String,
    /// `None` means the provider reported the window but not its usage.
    pub used_percent: Option<f64>,
    pub limit: i64,
    pub remaining: i64,
    pub resets_at_epoch: Option<i64>,
    pub resets_at_str: Option<String>,
    #[allow(dead_code)]
    pub has_real_remaining: bool,
    /// True only for a deliberately synthesized compatibility window.
    pub is_synthetic: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderResult {
    pub provider: ProviderId,
    pub name: String,
    pub status: ProviderStatus,
    pub error: Option<String>,
    pub plan: Option<String>,
    pub source: Option<FetchSource>,
    pub source_label: Option<String>,
    pub windows: Vec<QuotaWindow>,
    pub credits: Vec<CreditBalance>,
    pub metrics: Vec<UsageMetric>,
    pub fetched_at: chrono::DateTime<chrono::Local>,
}

impl ProviderResult {
    #[allow(dead_code)]
    pub fn auth_required(id: ProviderId) -> Self {
        Self {
            provider: id,
            name: id.display_name().to_string(),
            status: ProviderStatus::AuthRequired,
            error: Some("No credentials found".into()),
            plan: None,
            source: None,
            source_label: None,
            windows: vec![],
            credits: vec![],
            metrics: vec![],
            fetched_at: chrono::Local::now(),
        }
    }

    pub fn network_error(id: ProviderId, msg: impl Into<String>) -> Self {
        Self {
            provider: id,
            name: id.display_name().to_string(),
            status: ProviderStatus::NetworkError,
            error: Some(msg.into()),
            plan: None,
            source: None,
            source_label: None,
            windows: vec![],
            credits: vec![],
            metrics: vec![],
            fetched_at: chrono::Local::now(),
        }
    }

    pub fn unavailable(id: ProviderId, msg: impl Into<String>) -> Self {
        Self {
            provider: id,
            name: id.display_name().to_string(),
            status: ProviderStatus::Unavailable,
            error: Some(msg.into()),
            plan: None,
            source: None,
            source_label: None,
            windows: vec![],
            credits: vec![],
            metrics: vec![],
            fetched_at: chrono::Local::now(),
        }
    }
}
