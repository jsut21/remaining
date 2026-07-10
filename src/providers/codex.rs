use crate::engine::FetchOptions;
use crate::engine::snapshot::{
    CreditBalance, FetchSource, ProviderId, ProviderResult, ProviderStatus, QuotaUnit, QuotaWindow,
    WindowType,
};
use crate::utils::{network_error_message, read_home_file};
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Arc;
use std::time::Duration;

const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CODEX_RESET_CREDITS_URL: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";

pub async fn fetch(options: &FetchOptions) -> ProviderResult {
    let timeout = &options.timeout;
    let id = ProviderId::Codex;

    let auth_content = match read_home_file(".codex/auth.json") {
        Some(c) => c,
        None => {
            return ProviderResult {
                provider: id,
                name: "Codex".to_string(),
                status: ProviderStatus::AuthRequired,
                error: Some("No credentials found. Check ~/.codex/auth.json".to_string()),
                plan: None,
                source: Some(FetchSource::OAuthApi),
                source_label: Some("~/.codex/auth.json".to_string()),
                windows: vec![],
                credits: vec![],
                metrics: vec![],
                fetched_at: chrono::Local::now(),
            };
        }
    };

    let mut auth: serde_json::Value = match serde_json::from_str(&auth_content) {
        Ok(v) => v,
        Err(e) => {
            return ProviderResult::unavailable(id, format!("Invalid ~/.codex/auth.json: {}", e));
        }
    };
    let client = reqwest::Client::new();
    if let Ok(Some(refreshed)) =
        refresh_codex_auth_if_needed(&client, &auth, &auth_content, timeout).await
    {
        auth = refreshed;
    }

    let access_token = auth
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let account_id = auth
        .get("tokens")
        .and_then(|t| t.get("account_id"))
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);

    if access_token.is_empty() {
        return ProviderResult {
            provider: id,
            name: "Codex".to_string(),
            status: ProviderStatus::AuthRequired,
            error: Some("No access_token in ~/.codex/auth.json".into()),
            plan: None,
            source: Some(FetchSource::OAuthApi),
            source_label: Some("~/.codex/auth.json".to_string()),
            windows: vec![],
            credits: vec![],
            metrics: vec![],
            fetched_at: chrono::Local::now(),
        };
    }

    let mut request = client
        .get(CODEX_USAGE_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .header("User-Agent", "remaining")
        .timeout(*timeout.as_ref());

    if let Some(account_id) = account_id.as_deref() {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let resp = request.send().await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return ProviderResult::network_error(id, network_error_message(&e)),
    };

    let status = resp.status();
    let body_text = match resp.text().await {
        Ok(body) => body,
        Err(e) => return ProviderResult::unavailable(id, network_error_message(&e)),
    };

    if !status.is_success() {
        let message = extract_chatgpt_error_message(&body_text)
            .unwrap_or_else(|| format!("ChatGPT usage API returned HTTP {}", status.as_u16()));
        if status.as_u16() == 401 || status.as_u16() == 403 {
            let message = message.trim_end_matches(['.', '!', '?']);
            return ProviderResult {
                provider: id,
                name: "Codex".to_string(),
                status: ProviderStatus::AuthRequired,
                error: Some(format!(
                    "Codex ChatGPT auth failed: {}. Run `codex login` again.",
                    message
                )),
                plan: None,
                source: Some(FetchSource::OAuthApi),
                source_label: Some("wham/usage".to_string()),
                windows: vec![],
                credits: vec![],
                metrics: vec![],
                fetched_at: chrono::Local::now(),
            };
        }

        return ProviderResult::unavailable(id, message);
    }

    let body: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(b) => b,
        Err(e) => return ProviderResult::unavailable(id, e.to_string()),
    };

    let plan = body
        .get("plan_type")
        .and_then(|p| p.as_str())
        .unwrap_or("unknown")
        .to_string();
    let plan = format_chatgpt_plan(&plan);

    let mut windows = Vec::new();

    if let Some(rate_limit) = body.get("rate_limit") {
        if let Some(window) = rate_limit
            .get("primary_window")
            .and_then(|primary| codex_window_from_snapshot(primary, "5h", WindowType::FiveHours))
        {
            windows.push(window);
        }

        if let Some(window) = rate_limit.get("secondary_window").and_then(|secondary| {
            codex_window_from_snapshot(secondary, "Weekly", WindowType::Weekly)
        }) {
            windows.push(window);
        }
    }

    if let Some(limits) = body
        .get("additional_rate_limits")
        .and_then(|v| v.as_array())
    {
        let mut seen = HashSet::new();
        for limit in limits {
            append_codex_additional_windows(&mut windows, &mut seen, limit);
        }
    }

    if windows.is_empty() {
        return ProviderResult::unavailable(id, "No rate limit data found");
    }

    let credits = fetch_reset_credits(&client, &access_token, account_id.as_deref(), timeout).await;

    ProviderResult {
        provider: id,
        name: "Codex".to_string(),
        status: ProviderStatus::Available,
        error: None,
        plan: Some(plan),
        source: Some(FetchSource::OAuthApi),
        source_label: Some("wham/usage".to_string()),
        windows,
        credits,
        metrics: vec![],
        fetched_at: chrono::Local::now(),
    }
}

async fn refresh_codex_auth_if_needed(
    client: &reqwest::Client,
    auth: &serde_json::Value,
    original_content: &str,
    timeout: &Arc<Duration>,
) -> Result<Option<serde_json::Value>, String> {
    if !codex_auth_needs_refresh(auth) {
        return Ok(None);
    }
    let refresh_token = auth
        .get("tokens")
        .and_then(|tokens| {
            tokens
                .get("refresh_token")
                .or_else(|| tokens.get("refreshToken"))
        })
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty());
    let Some(refresh_token) = refresh_token else {
        return Ok(None);
    };

    let response = client
        .post("https://auth.openai.com/oauth/token")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "scope": "openid profile email"
        }))
        .timeout(*timeout.as_ref())
        .send()
        .await
        .map_err(|error| {
            format!(
                "Codex OAuth refresh failed: {}",
                network_error_message(&error)
            )
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|error| {
        format!(
            "Codex OAuth refresh response failed: {}",
            network_error_message(&error)
        )
    })?;
    if !status.is_success() {
        return Err(format!(
            "Codex OAuth refresh returned HTTP {}",
            status.as_u16()
        ));
    }
    let refreshed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("Invalid Codex OAuth refresh response: {}", error))?;
    let access_token = refreshed
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "Codex OAuth refresh response had no access token".to_string())?;

    let mut updated = auth.clone();
    let tokens = updated
        .get_mut("tokens")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| "Codex auth.json has no token object".to_string())?;
    tokens.insert(
        "access_token".to_string(),
        serde_json::Value::String(access_token.to_string()),
    );
    if let Some(value) = refreshed
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        tokens.insert(
            "refresh_token".to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
    if let Some(value) = refreshed
        .get("id_token")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        tokens.insert(
            "id_token".to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
    updated["last_refresh"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());

    let path = dirs::home_dir()
        .ok_or_else(|| "Could not resolve home directory".to_string())?
        .join(".codex/auth.json");
    if let Ok(current) = std::fs::read_to_string(&path) {
        if current != original_content {
            return serde_json::from_str(&current)
                .map(Some)
                .map_err(|error| format!("Codex auth.json changed during refresh: {}", error));
        }
    }
    write_private_json(&path, &updated)?;
    Ok(Some(updated))
}

fn codex_auth_needs_refresh(auth: &serde_json::Value) -> bool {
    let Some(last_refresh) = auth
        .get("last_refresh")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return true;
    };
    chrono::Utc::now().signed_duration_since(last_refresh.with_timezone(&chrono::Utc))
        > chrono::Duration::days(8)
}

fn write_private_json(path: &std::path::Path, value: &serde_json::Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Codex auth.json has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create Codex directory: {}", error))?;
    let temporary = parent.join(format!(
        ".auth.json.remaining-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| format!("Could not stage Codex auth.json: {}", error))?;
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|error| format!("Could not encode Codex auth.json: {}", error))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Could not write Codex auth.json: {}", error))?;
        std::fs::rename(&temporary, path)
            .map_err(|error| format!("Could not replace Codex auth.json: {}", error))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn codex_window_from_snapshot(
    snapshot: &serde_json::Value,
    label: &str,
    wtype: WindowType,
) -> Option<QuotaWindow> {
    let used_pct = first_f64(snapshot, &["used_percent", "usedPercent"])?.clamp(0.0, 100.0);
    let reset_at = first_epoch(snapshot, &["reset_at", "resetAt"]).or_else(|| {
        first_i64(snapshot, &["reset_after_seconds", "resetAfterSeconds"])
            .map(|secs| chrono::Utc::now().timestamp() + secs)
    });

    Some(QuotaWindow {
        wtype,
        label: label.to_string(),
        used_percent: Some(used_pct),
        limit: 100,
        remaining: (100.0 - used_pct).clamp(0.0, 100.0).round() as i64,
        resets_at_epoch: reset_at,
        resets_at_str: None,
        has_real_remaining: false,
        is_synthetic: false,
    })
}

fn append_codex_additional_windows(
    windows: &mut Vec<QuotaWindow>,
    seen: &mut HashSet<(WindowType, String)>,
    limit: &serde_json::Value,
) {
    let label_text = first_string(
        limit,
        &[
            "limit_name",
            "limitName",
            "metered_feature",
            "meteredFeature",
            "limit_id",
            "limitId",
        ],
    )
    .unwrap_or_else(|| "Additional".to_string());
    let is_spark = label_text.to_ascii_lowercase().contains("spark");

    if let Some(rate_limit) = limit.get("rate_limit").or_else(|| limit.get("rateLimit")) {
        let primary = rate_limit
            .get("primary_window")
            .or_else(|| rate_limit.get("primaryWindow"));
        let secondary = rate_limit
            .get("secondary_window")
            .or_else(|| rate_limit.get("secondaryWindow"));
        if is_spark {
            for (snapshot, fallback) in [(primary, None), (secondary, Some(WindowType::Weekly))] {
                if let Some(window) =
                    snapshot.and_then(|value| codex_additional_window(value, &label_text, fallback))
                {
                    push_unique_window(windows, seen, window);
                }
            }
        } else if let Some(window) = primary
            .or(secondary)
            .and_then(|value| codex_additional_window(value, &label_text, None))
        {
            push_unique_window(windows, seen, window);
        }
        return;
    }

    if let Some(window) = codex_additional_window(limit, &label_text, None) {
        push_unique_window(windows, seen, window);
    }
}

fn push_unique_window(
    windows: &mut Vec<QuotaWindow>,
    seen: &mut HashSet<(WindowType, String)>,
    window: QuotaWindow,
) {
    if seen.insert((window.wtype, window.label.clone())) {
        windows.push(window);
    }
}

fn codex_additional_window(
    snapshot: &serde_json::Value,
    label_text: &str,
    fallback_type: Option<WindowType>,
) -> Option<QuotaWindow> {
    let window_secs =
        first_i64(snapshot, &["limit_window_seconds", "limitWindowSeconds"]).unwrap_or(0);
    let (wtype, label) =
        if matches!(fallback_type, Some(WindowType::Weekly)) || window_secs >= 6 * 24 * 60 * 60 {
            (WindowType::Weekly, format!("Weekly ({label_text})"))
        } else if window_secs > 0 && window_secs <= 6 * 60 * 60 {
            (WindowType::FiveHours, format!("5h ({label_text})"))
        } else {
            (
                fallback_type.unwrap_or(WindowType::Other),
                label_text.to_string(),
            )
        };
    codex_window_from_snapshot(snapshot, &label, wtype)
}

async fn fetch_reset_credits(
    client: &reqwest::Client,
    access_token: &str,
    account_id: Option<&str>,
    timeout: &Arc<Duration>,
) -> Vec<CreditBalance> {
    let mut request = client
        .get(CODEX_RESET_CREDITS_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .header("User-Agent", "remaining")
        .header("OpenAI-Beta", "codex-1")
        .header("originator", "Codex Desktop")
        .timeout(*timeout.as_ref());

    if let Some(account_id) = account_id {
        request = request.header("ChatGPT-Account-ID", account_id);
    }

    let Ok(resp) = request.send().await else {
        return vec![];
    };
    if !resp.status().is_success() {
        return vec![];
    }
    let Ok(text) = resp.text().await else {
        return vec![];
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return vec![];
    };

    parse_credit_balances(&value)
}

fn parse_credit_balances(value: &serde_json::Value) -> Vec<CreditBalance> {
    let mut credits = Vec::new();
    collect_credit_balances(value, &mut credits);
    credits
}

fn collect_credit_balances(value: &serde_json::Value, out: &mut Vec<CreditBalance>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(credit) = credit_from_object(item) {
                    out.push(credit);
                } else {
                    collect_credit_balances(item, out);
                }
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(credit) = credit_from_object(value) {
                out.push(credit);
                return;
            }
            for key in [
                "credits",
                "data",
                "items",
                "reset_credits",
                "rate_limit_reset_credits",
                "available_credits",
            ] {
                if let Some(child) = map.get(key) {
                    collect_credit_balances(child, out);
                }
            }
        }
        _ => {}
    }
}

fn credit_from_object(value: &serde_json::Value) -> Option<CreditBalance> {
    let object = value.as_object()?;

    if object
        .get("redeemed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || object.get("redeemed_at").is_some_and(|v| !v.is_null())
    {
        return None;
    }

    let status = first_string(value, &["status"]);
    if status
        .as_deref()
        .is_some_and(|status| !matches!(status, "available" | "active" | "unused"))
    {
        return None;
    }

    let mut remaining = first_i64(
        value,
        &[
            "remaining",
            "available",
            "count",
            "credits",
            "quantity",
            "balance",
        ],
    );
    let mut total = first_i64(value, &["total", "limit", "initial", "granted"]);
    let expires_at_epoch = first_epoch(
        value,
        &[
            "expires_at_epoch",
            "expiresAtEpoch",
            "expires_at",
            "expiresAt",
            "expiration_time",
            "expirationTime",
        ],
    );

    let looks_like_single_reset_credit = expires_at_epoch.is_some()
        && (object.contains_key("reset_type")
            || object.contains_key("title")
            || object.contains_key("granted_at")
            || object.contains_key("redeem_started_at"));

    if remaining.is_none() && total.is_none() {
        if looks_like_single_reset_credit {
            remaining = Some(1);
            total = Some(1);
        } else {
            return None;
        }
    }

    let label = first_string(
        value,
        &[
            "title",
            "label",
            "name",
            "reset_type",
            "type",
            "credit_type",
            "window",
            "description",
        ],
    )
    .unwrap_or_else(|| "Reset credits".to_string());

    Some(CreditBalance {
        label,
        remaining,
        total,
        unit: QuotaUnit::Credits,
        expires_at_epoch,
        source_label: Some("rate-limit-reset-credits".to_string()),
    })
}

fn first_i64(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(found) = value.get(*key).and_then(value_to_i64) {
            return Some(found);
        }
    }
    None
}

fn first_f64(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        let value = value.get(*key)?;
        value
            .as_f64()
            .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
            .filter(|number| number.is_finite())
    })
}

fn first_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = value
            .get(*key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Some(found.to_string());
        }
    }
    None
}

fn first_epoch(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        let Some(raw) = value.get(*key) else {
            continue;
        };
        if let Some(epoch) = value_to_i64(raw) {
            return Some(if epoch > 10_000_000_000 {
                epoch / 1000
            } else {
                epoch
            });
        }
        if let Some(dt) = raw
            .as_str()
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        {
            return Some(dt.timestamp());
        }
    }
    None
}

fn value_to_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
        .or_else(|| value.as_f64().map(|v| v.round() as i64))
        .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
}

fn format_chatgpt_plan(plan: &str) -> String {
    match plan {
        "plus" => "ChatGPT Plus".to_string(),
        "pro" => "ChatGPT Pro".to_string(),
        "team" => "ChatGPT Team".to_string(),
        "enterprise" => "ChatGPT Enterprise".to_string(),
        "free" => "ChatGPT Free".to_string(),
        other => other.to_string(),
    }
}

fn extract_chatgpt_error_message(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    json.get("error")
        .and_then(|error| error.get("message"))
        .and_then(|message| message.as_str())
        .or_else(|| json.get("message").and_then(|message| message.as_str()))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_reset_credit_items_without_count_fields() {
        let value = serde_json::json!({
            "credits": [
                {
                    "id": "redacted",
                    "reset_type": "codex_rate_limits",
                    "status": "available",
                    "granted_at": "2026-06-27T00:08:29.879961Z",
                    "expires_at": "2026-07-27T00:08:29.879961Z",
                    "redeem_started_at": null,
                    "redeemed_at": null,
                    "title": "Full reset (Weekly + 5 hr)",
                    "description": "Thanks for using Codex! You've been granted one free rate limit reset."
                },
                {
                    "id": "redeemed",
                    "reset_type": "codex_rate_limits",
                    "status": "redeemed",
                    "expires_at": "2026-07-27T00:08:29.879961Z",
                    "redeemed_at": "2026-07-01T00:00:00Z",
                    "title": "Full reset (Weekly + 5 hr)"
                }
            ],
            "available_count": 1
        });

        let credits = parse_credit_balances(&value);
        assert_eq!(credits.len(), 1);
        assert_eq!(credits[0].label, "Full reset (Weekly + 5 hr)");
        assert_eq!(credits[0].remaining, Some(1));
        assert_eq!(credits[0].total, Some(1));
        assert_eq!(credits[0].expires_at_epoch, Some(1785110909));
    }

    #[test]
    fn maps_generic_limit_once_and_spark_to_two_deduplicated_windows() {
        let limits: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/codex/additional-rate-limits.json"
        ))
        .unwrap();
        let mut windows = Vec::new();
        let mut seen = HashSet::new();
        for limit in limits.as_array().unwrap() {
            append_codex_additional_windows(&mut windows, &mut seen, limit);
        }

        assert_eq!(windows.len(), 3);
        assert_eq!(
            windows
                .iter()
                .filter(|window| window.label.contains("Model A"))
                .count(),
            1
        );
        assert_eq!(
            windows
                .iter()
                .filter(|window| window.label.contains("Spark"))
                .count(),
            2
        );
    }

    #[test]
    fn refresh_age_matches_eight_day_policy() {
        let recent = serde_json::json!({
            "last_refresh": chrono::Utc::now().to_rfc3339()
        });
        let stale = serde_json::json!({
            "last_refresh": (chrono::Utc::now() - chrono::Duration::days(9)).to_rfc3339()
        });

        assert!(!codex_auth_needs_refresh(&recent));
        assert!(codex_auth_needs_refresh(&stale));
        assert!(codex_auth_needs_refresh(&serde_json::json!({})));
    }
}
