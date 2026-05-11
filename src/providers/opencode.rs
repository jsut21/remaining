use crate::types::{ProviderId, ProviderResult, ProviderStatus, QuotaWindow, WindowType};
use crate::utils::read_home_file;
use std::sync::Arc;
use std::time::Duration;

pub async fn fetch(timeout: &Arc<Duration>) -> ProviderResult {
    let id = ProviderId::OpenCode;

    let auth_content = match read_home_file(".local/share/opencode/auth.json") {
        Some(c) => c,
        None => return ProviderResult {
            provider: id,
            name: "OpenCode".to_string(),
            status: ProviderStatus::AuthRequired,
            error: Some("No credentials found. Check ~/.local/share/opencode/auth.json".into()),
            plan: None,
            windows: vec![],
            fetched_at: chrono::Local::now(),
        },
    };

    let auth: serde_json::Value = match serde_json::from_str(&auth_content) {
        Ok(v) => v,
        Err(e) => return ProviderResult::unavailable(id, format!("Invalid ~/.local/share/opencode/auth.json: {}", e)),
    };

    let root = match auth.as_object() {
        Some(obj) => obj,
        None => return ProviderResult::unavailable(id, "~/.local/share/opencode/auth.json is not an object"),
    };

    let mut windows = Vec::new();
    let mut plan_name = String::new();
    let mut found_any = false;

    if let Some(openai) = root.get("openai") {
        if openai.get("type").and_then(|t| t.as_str()) == Some("oauth") {
            let access_token = openai
                .get("access")
                .and_then(|t| t.as_str())
                .unwrap_or("");

            if !access_token.is_empty() {
                if let Some(result) = fetch_codex_usage(access_token, timeout).await {
                    plan_name = result.plan.clone().unwrap_or_default();
                    windows = result.windows;
                    found_any = true;
                }
            }
        }
    }

    if !found_any {
        return ProviderResult {
            provider: id,
            name: "OpenCode".to_string(),
            status: ProviderStatus::AuthRequired,
            error: Some("No OpenAI OAuth token in ~/.local/share/opencode/auth.json".into()),
            plan: None,
            windows: vec![],
            fetched_at: chrono::Local::now(),
        };
    }

    ProviderResult {
        provider: id,
        name: "OpenCode".to_string(),
        status: ProviderStatus::Available,
        error: None,
        plan: if plan_name.is_empty() { None } else { Some(plan_name) },
        windows,
        fetched_at: chrono::Local::now(),
    }
}

async fn fetch_codex_usage(access_token: &str, timeout: &Arc<Duration>) -> Option<ProviderResult> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("Authorization", format!("Bearer {}", access_token))
        .timeout(*timeout.as_ref())
        .send()
        .await
        .ok()?;

    let body: serde_json::Value = resp.json().await.ok()?;

    let plan = body
        .get("plan_type")
        .and_then(|p| p.as_str())
        .unwrap_or("unknown")
        .to_string();

    let mut windows = Vec::new();

    if let Some(rate_limit) = body.get("rate_limit") {
        if let Some(primary) = rate_limit.get("primary_window") {
            let used_pct = primary
                .get("used_percent")
                .and_then(|u| u.as_i64())
                .unwrap_or(0) as i32;
            let reset_at = primary
                .get("reset_at")
                .and_then(|r| r.as_i64())
                .filter(|e| *e > 0);
            let reset_after_secs = primary
                .get("reset_after_seconds")
                .and_then(|r| r.as_i64())
                .unwrap_or(0);

            let effective_reset = reset_at.unwrap_or_else(|| {
                chrono::Utc::now().timestamp() + reset_after_secs
            });

            windows.push(QuotaWindow {
                wtype: WindowType::FiveHours,
                label: "5h".to_string(),
                used_percent: used_pct,
                limit: 100,
                remaining: (100 - used_pct) as i64,
                resets_at_epoch: Some(effective_reset),
                resets_at_str: None,
                has_real_remaining: false,
            });
        }

        if let Some(secondary) = rate_limit.get("secondary_window") {
            let used_pct = secondary
                .get("used_percent")
                .and_then(|u| u.as_i64())
                .unwrap_or(0) as i32;
            let reset_at = secondary
                .get("reset_at")
                .and_then(|r| r.as_i64())
                .filter(|e| *e > 0);
            let reset_after_secs = secondary
                .get("reset_after_seconds")
                .and_then(|r| r.as_i64())
                .unwrap_or(0);

            let effective_reset = reset_at.unwrap_or_else(|| {
                chrono::Utc::now().timestamp() + reset_after_secs
            });

            windows.push(QuotaWindow {
                wtype: WindowType::Weekly,
                label: "Weekly".to_string(),
                used_percent: used_pct,
                limit: 100,
                remaining: (100 - used_pct) as i64,
                resets_at_epoch: Some(effective_reset),
                resets_at_str: None,
                has_real_remaining: false,
            });
        }
    }

    if windows.is_empty() {
        return None;
    }

    Some(ProviderResult {
        provider: ProviderId::OpenCode,
        name: "OpenCode".to_string(),
        status: ProviderStatus::Available,
        error: None,
        plan: Some(plan),
        windows,
        fetched_at: chrono::Local::now(),
    })
}