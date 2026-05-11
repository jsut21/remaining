use crate::types::{ProviderId, ProviderResult, ProviderStatus, QuotaWindow, WindowType};
use crate::utils::read_home_file;
use std::sync::Arc;
use std::time::Duration;

pub async fn fetch(timeout: &Arc<Duration>) -> ProviderResult {
    let id = ProviderId::Zai;

    let api_key = std::env::var("ZAI_API_KEY")
        .or_else(|_| std::env::var("ZHIPU_API_KEY"))
        .ok()
        .or_else(|| read_home_file(".api-zai"))
        .or_else(|| read_opencode_zai_key());

    let api_key = match api_key {
        Some(k) => k.trim().to_string(),
        None => return ProviderResult {
            provider: id,
            name: "Z.ai".to_string(),
            status: ProviderStatus::AuthRequired,
            error: Some("No API key found. Set ZAI_API_KEY env var, ~/.api-zai, or ~/.local/share/opencode/auth.json".into()),
            plan: None,
            windows: vec![],
            fetched_at: chrono::Local::now(),
        },
    };

    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.z.ai/api/monitor/usage/quota/limit")
        .header("Authorization", format!("Bearer {}", api_key))
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

    let code = body.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
    if code != 200 {
        return ProviderResult::unavailable(id, format!("API error code: {}", code));
    }

    let data = body.get("data").unwrap_or(&body);
    let plan = data
        .get("level")
        .and_then(|l| l.as_str())
        .unwrap_or("unknown")
        .to_string();

    let limits = match data.get("limits").and_then(|l| l.as_array()) {
        Some(arr) => arr,
        None => return ProviderResult::unavailable(id, "No limits data"),
    };

    let mut windows = Vec::new();

    for limit in limits {
        let ltype = limit
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown");
        let pct = limit
            .get("percentage")
            .and_then(|p| p.as_i64())
            .unwrap_or(0) as i32;
        let remaining = limit
            .get("remaining")
            .and_then(|r| r.as_i64())
            .unwrap_or(0);
        let usage = limit
            .get("usage")
            .and_then(|u| u.as_i64())
            .unwrap_or(0);
        let next_reset = limit
            .get("nextResetTime")
            .and_then(|n| n.as_i64())
            .filter(|n| *n > 0);
        let unit = limit.get("unit").and_then(|u| u.as_i64()).unwrap_or(0);

        let (wtype, label) = match ltype {
            "TOKENS_LIMIT" => {
                match unit {
                    3 => (WindowType::FiveHours, "5h".to_string()),
                    6 => (WindowType::Weekly, "Weekly".to_string()),
                    _ => (WindowType::Other, "Tokens".to_string()),
                }
            }
            "TIME_LIMIT" => (WindowType::Monthly, "Monthly tools".to_string()),
            _ => (WindowType::Other, ltype.to_string()),
        };

        windows.push(QuotaWindow {
            wtype,
            label,
            used_percent: pct,
            limit: usage + remaining,
            remaining,
            resets_at_epoch: next_reset.map(|e| e / 1000),
            resets_at_str: None,
            has_real_remaining: true,
        });
    }

    ProviderResult {
        provider: id,
        name: "Z.ai".to_string(),
        status: ProviderStatus::Available,
        error: None,
        plan: Some(plan),
        windows,
        fetched_at: chrono::Local::now(),
    }
}

fn read_opencode_zai_key() -> Option<String> {
    let content = read_home_file(".local/share/opencode/auth.json")?;
    let auth: serde_json::Value = serde_json::from_str(&content).ok()?;
    auth.get("zai-coding-plan")
        .and_then(|e| e.get("key"))
        .and_then(|k| k.as_str())
        .map(|s| s.trim().to_string())
}