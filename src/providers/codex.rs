use crate::types::{ProviderId, ProviderResult, ProviderStatus, QuotaWindow, WindowType};
use crate::utils::read_home_file;
use std::sync::Arc;
use std::time::Duration;

pub async fn fetch(timeout: &Arc<Duration>) -> ProviderResult {
    let id = ProviderId::Codex;

    let auth_path = dirs::home_dir().map(|h| h.join(".codex/auth.json"));
    let auth_content = match read_home_file(".codex/auth.json") {
        Some(c) => c,
        None => return ProviderResult {
            provider: id,
            name: "Codex".to_string(),
            status: ProviderStatus::AuthRequired,
            error: Some(format!("No credentials found. Check {}", auth_path.map(|p| p.display().to_string()).unwrap_or_else(|| "~/.codex/auth.json".into()))),
            plan: None,
            windows: vec![],
            fetched_at: chrono::Local::now(),
        },
    };

    let auth: serde_json::Value = match serde_json::from_str(&auth_content) {
        Ok(v) => v,
        Err(e) => return ProviderResult::unavailable(id, format!("Invalid ~/.codex/auth.json: {}", e)),
    };

    let access_token = auth
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    if access_token.is_empty() {
        return ProviderResult {
            provider: id,
            name: "Codex".to_string(),
            status: ProviderStatus::AuthRequired,
            error: Some("No access_token in ~/.codex/auth.json".into()),
            plan: None,
            windows: vec![],
            fetched_at: chrono::Local::now(),
        };
    }

    let client = reqwest::Client::new();
    let resp = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("Authorization", format!("Bearer {}", access_token))
        .timeout(*timeout.as_ref())
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return ProviderResult::network_error(id, e.to_string()),
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => return ProviderResult::unavailable(id, e.to_string()),
    };

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

    if let Some(additional) = body.get("additional_rate_limits") {
        if let Some(limits) = additional.as_array() {
            for limit in limits {
                let used_pct = limit
                    .get("used_percent")
                    .and_then(|u| u.as_i64())
                    .unwrap_or(0) as i32;
                let window_secs = limit
                    .get("limit_window_seconds")
                    .and_then(|w| w.as_i64())
                    .unwrap_or(0);
                let label_text = limit
                    .get("limit_id")
                    .and_then(|l| l.as_str())
                    .unwrap_or("Additional");

                let (wtype, label) = if window_secs > 0 && window_secs <= 18000 {
                    (WindowType::FiveHours, format!("5h ({})", label_text))
                } else if window_secs > 18000 {
                    (WindowType::Weekly, format!("Weekly ({})", label_text))
                } else {
                    (WindowType::Other, label_text.to_string())
                };

                let reset_at = limit
                    .get("reset_at")
                    .and_then(|r| r.as_i64())
                    .filter(|e| *e > 0);

                windows.push(QuotaWindow {
                    wtype,
                    label,
                    used_percent: used_pct,
                    limit: 100,
                    remaining: (100 - used_pct) as i64,
                    resets_at_epoch: reset_at,
                    resets_at_str: None,
                    has_real_remaining: false,
                });
            }
        }
    }

    if windows.is_empty() {
        return ProviderResult::unavailable(id, "No rate limit data found");
    }

    ProviderResult {
        provider: id,
        name: "Codex".to_string(),
        status: ProviderStatus::Available,
        error: None,
        plan: Some(plan),
        windows,
        fetched_at: chrono::Local::now(),
    }
}