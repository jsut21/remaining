use super::FetchOptions;
use super::snapshot::{ProviderId, ProviderResult, WindowType};
use crate::providers;

#[derive(Debug, Clone, Copy)]
pub struct ProviderDescriptor {
    pub id: ProviderId,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub default_enabled: bool,
    pub auth_methods: &'static [&'static str],
    pub windows: &'static [WindowType],
}

pub const PROVIDERS: &[ProviderDescriptor] = &[
    ProviderDescriptor {
        id: ProviderId::Codex,
        display_name: "Codex",
        aliases: &["codex", "chatgpt"],
        default_enabled: true,
        auth_methods: &["~/.codex/auth.json"],
        windows: &[WindowType::FiveHours, WindowType::Weekly],
    },
    ProviderDescriptor {
        id: ProviderId::Zai,
        display_name: "Z.ai",
        aliases: &["zai", "z.ai", "zhipu"],
        default_enabled: true,
        auth_methods: &[
            "ZAI_API_KEY",
            "ZHIPU_API_KEY",
            "~/.api-zai",
            "opencode auth.json",
        ],
        windows: &[
            WindowType::FiveHours,
            WindowType::Weekly,
            WindowType::Monthly,
        ],
    },
    ProviderDescriptor {
        id: ProviderId::Ollama,
        display_name: "Ollama Cloud",
        aliases: &["ollama", "ollama-cloud"],
        default_enabled: true,
        auth_methods: &[
            "OLLAMA_SESSION_COOKIE",
            "~/.config/ollama-usage/session",
            "Firefox cookies (default)",
        ],
        windows: &[WindowType::FiveHours, WindowType::Weekly],
    },
    ProviderDescriptor {
        id: ProviderId::OpenCode,
        display_name: "OpenCode Go",
        aliases: &["opencode-go", "opencode", "go"],
        default_enabled: false,
        auth_methods: &[
            "OPENCODE_GO_WORKSPACE_ID",
            "OPENCODE_GO_AUTH_COOKIE",
            "~/.config/opencode-quota/opencode-go.json",
            "Chrome cookies (default)",
        ],
        windows: &[
            WindowType::FiveHours,
            WindowType::Weekly,
            WindowType::Monthly,
        ],
    },
    ProviderDescriptor {
        id: ProviderId::MiniMax,
        display_name: "MiniMax",
        aliases: &["minimax"],
        default_enabled: false,
        auth_methods: &[
            "MINIMAX_API_KEY",
            "MINIMAX_SUBSCRIPTION_KEY",
            "~/.api-minimax",
            "Firefox cookies (default)",
        ],
        windows: &[WindowType::FiveHours, WindowType::Weekly],
    },
];

pub fn default_provider_ids() -> Vec<ProviderId> {
    PROVIDERS
        .iter()
        .filter(|provider| provider.default_enabled)
        .map(|provider| provider.id)
        .collect()
}

pub fn all_provider_ids() -> Vec<ProviderId> {
    PROVIDERS.iter().map(|provider| provider.id).collect()
}

pub fn provider_ids_from_specs(specs: &[String]) -> Vec<ProviderId> {
    if specs.is_empty() {
        return default_provider_ids();
    }

    let mut seen = std::collections::HashSet::new();
    specs
        .iter()
        .filter_map(|spec| find_by_alias(spec).map(|provider| provider.id))
        .filter(|provider| seen.insert(*provider))
        .collect()
}

pub fn provider_list_help() -> String {
    PROVIDERS
        .iter()
        .map(|provider| provider.aliases[0])
        .collect::<Vec<_>>()
        .join(",")
}

pub fn find_by_alias(alias: &str) -> Option<&'static ProviderDescriptor> {
    let normalized = alias.trim().to_ascii_lowercase();
    PROVIDERS.iter().find(|provider| {
        provider
            .aliases
            .iter()
            .any(|candidate| *candidate == normalized)
    })
}

pub async fn fetch_provider(id: ProviderId, options: &FetchOptions) -> ProviderResult {
    match id {
        ProviderId::Zai => providers::zai::fetch(options).await,
        ProviderId::Codex => providers::codex::fetch(options).await,
        ProviderId::OpenCode => providers::opencode::fetch(options).await,
        ProviderId::Ollama => providers::ollama::fetch(options).await,
        ProviderId::MiniMax => providers::minimax::fetch(options).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_provider_specs_are_detectable() {
        assert!(find_by_alias("codex").is_some());
        assert!(find_by_alias("definitely-not-a-provider").is_none());
    }

    #[test]
    fn aliases_do_not_duplicate_provider_fetches() {
        let specs = vec!["codex".to_string(), "chatgpt".to_string()];
        assert_eq!(provider_ids_from_specs(&specs), vec![ProviderId::Codex]);
    }
}
