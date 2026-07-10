use crate::engine::FetchOptions;
use crate::engine::snapshot::{
    FetchSource, ProviderId, ProviderResult, ProviderStatus, QuotaUnit, QuotaWindow, UsageMetric,
    WindowType,
};
use crate::utils::{network_error_message, read_home_file};

const DEFAULT_ZAI_HOST: &str = "api.z.ai";
const DEFAULT_ZAI_PATH: &str = "/api/monitor/usage/quota/limit";

pub async fn fetch(options: &FetchOptions) -> ProviderResult {
    let id = ProviderId::Zai;
    let timeout = &options.timeout;

    let config = match resolve_config(options.allow_custom_endpoints) {
        Ok(config) => config,
        Err(error) => {
            return ProviderResult {
                provider: id,
                name: "Z.ai".to_string(),
                status: error.status,
                error: Some(error.message),
                plan: None,
                source: Some(FetchSource::ApiKey),
                source_label: None,
                windows: vec![],
                credits: vec![],
                metrics: vec![],
                fetched_at: chrono::Local::now(),
            };
        }
    };

    let client = reqwest::Client::new();
    let mut request = client
        .get(&config.quota_url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Accept", "application/json")
        .timeout(*timeout.as_ref());

    if let Some(org) = config.organization_id.as_deref() {
        request = request.header("Bigmodel-Organization", org);
    }
    if let Some(project) = config.project_id.as_deref() {
        request = request.header("Bigmodel-Project", project);
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
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return ProviderResult {
            provider: id,
            name: "Z.ai".to_string(),
            status: ProviderStatus::AuthRequired,
            error: Some(format!(
                "Z.ai API authentication failed (HTTP {}). Check the configured API key.",
                status.as_u16()
            )),
            plan: None,
            source: Some(FetchSource::ApiKey),
            source_label: Some(config.source_label),
            windows: vec![],
            credits: vec![],
            metrics: vec![],
            fetched_at: chrono::Local::now(),
        };
    }
    if !status.is_success() {
        return ProviderResult::unavailable(
            id,
            format!("Z.ai API returned HTTP {}", status.as_u16()),
        );
    }

    let body: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(b) => b,
        Err(e) => return ProviderResult::unavailable(id, e.to_string()),
    };

    let code = body.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
    let success = body
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if code != 200 || !success {
        let message = body
            .get("msg")
            .and_then(serde_json::Value::as_str)
            .filter(|message| !message.trim().is_empty())
            .unwrap_or("request was not successful");
        return ProviderResult::unavailable(id, format!("Z.ai API error: {} ({})", message, code));
    }

    let data = body.get("data").unwrap_or(&body);
    let plan = first_string(
        data,
        &["planName", "plan", "plan_type", "packageName", "level"],
    )
    .unwrap_or_else(|| "unknown".to_string());

    let empty_limits = Vec::new();
    let limits = data
        .get("limits")
        .and_then(|limits| limits.as_array())
        .unwrap_or(&empty_limits);

    let mut token_windows = Vec::new();
    let mut other_windows = Vec::new();

    for limit in limits {
        if let Some(window) = zai_window_from_limit(limit) {
            if limit.get("type").and_then(|t| t.as_str()) == Some("TOKENS_LIMIT") {
                token_windows.push(window);
            } else {
                other_windows.push(window);
            }
        }
    }

    token_windows.sort_by_key(|window| match window.wtype {
        WindowType::FiveHours => 0,
        WindowType::Weekly => 1,
        WindowType::Monthly => 2,
        WindowType::Other => 3,
    });

    let mut windows = Vec::new();
    windows.extend(token_windows);
    windows.extend(other_windows);
    let metrics = usage_detail_metrics(limits, &config.source_label);
    if windows.is_empty() {
        return ProviderResult::unavailable(id, "No quota limits found in Z.ai response");
    }

    ProviderResult {
        provider: id,
        name: "Z.ai".to_string(),
        status: ProviderStatus::Available,
        error: None,
        plan: Some(plan),
        source: Some(FetchSource::ApiKey),
        source_label: Some(config.source_label),
        windows,
        credits: vec![],
        metrics,
        fetched_at: chrono::Local::now(),
    }
}

fn zai_window_from_limit(limit: &serde_json::Value) -> Option<QuotaWindow> {
    let ltype = limit
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("unknown");
    let unit = value_to_i64(limit.get("unit")).unwrap_or(0);
    let number = value_to_i64(limit.get("number"))
        .or_else(|| value_to_i64(limit.get("num")))
        .unwrap_or(0);
    let pct = value_to_f64(limit.get("percentage")).map(|value| value.clamp(0.0, 100.0));
    let usage_limit = value_to_i64(limit.get("usage"));
    let current_value = value_to_i64(limit.get("currentValue"));
    let remaining_units = value_to_i64(limit.get("remaining"));
    let next_reset = value_to_i64(limit.get("nextResetTime")).filter(|n| *n > 0);

    let (wtype, label) = match ltype {
        "TOKENS_LIMIT" => match (unit, number) {
            (3, n) if n <= 6 => (WindowType::FiveHours, format!("{}h", n.max(1))),
            (6, _) => (WindowType::Weekly, "Weekly".to_string()),
            (_, n) if n > 0 => (WindowType::Other, format!("Tokens {}", n)),
            _ => (WindowType::Other, "Tokens".to_string()),
        },
        "TIME_LIMIT" => (WindowType::Monthly, "Monthly tools".to_string()),
        _ => (WindowType::Other, ltype.to_string()),
    };

    let (used_percent, limit_total, remaining, has_real_remaining) =
        if let Some(limit_total) = usage_limit.filter(|limit| *limit > 0) {
            let used_from_remaining = remaining_units.map(|remaining| limit_total - remaining);
            let used_raw = match (used_from_remaining, current_value) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            if let Some(used_raw) = used_raw {
                let used = used_raw.clamp(0, limit_total);
                (
                    Some((used as f64 * 100.0 / limit_total as f64).clamp(0.0, 100.0)),
                    limit_total,
                    remaining_units
                        .unwrap_or((limit_total - used).max(0))
                        .clamp(0, limit_total),
                    true,
                )
            } else {
                (
                    pct,
                    pct.map(|_| 100).unwrap_or(0),
                    pct.map(|value| (100.0 - value).round() as i64).unwrap_or(0),
                    false,
                )
            }
        } else {
            (
                pct,
                pct.map(|_| 100).unwrap_or(0),
                pct.map(|value| (100.0 - value).round() as i64).unwrap_or(0),
                false,
            )
        };

    Some(QuotaWindow {
        wtype,
        label,
        used_percent,
        limit: limit_total,
        remaining,
        resets_at_epoch: next_reset.map(|e| if e > 10_000_000_000 { e / 1000 } else { e }),
        resets_at_str: None,
        has_real_remaining,
        is_synthetic: false,
    })
}

fn usage_detail_metrics(limits: &[serde_json::Value], source: &str) -> Vec<UsageMetric> {
    let mut metrics = Vec::new();
    for limit in limits {
        let period = zai_window_from_limit(limit).map(|window| window.label);
        let Some(details) = limit
            .get("usageDetails")
            .or_else(|| limit.get("usage_details"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for detail in details {
            let Some(label) = first_string(detail, &["modelCode", "model_code"]) else {
                continue;
            };
            let Some(value) = value_to_f64(detail.get("usage")) else {
                continue;
            };
            metrics.push(UsageMetric {
                label,
                value,
                unit: QuotaUnit::Units,
                period: period.clone(),
                source_label: Some(source.to_string()),
            });
        }
    }
    metrics
}

fn value_to_i64(value: Option<&serde_json::Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
        .or_else(|| value.as_f64().map(|v| v.round() as i64))
        .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
}

fn value_to_f64(value: Option<&serde_json::Value>) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

struct ZaiConfigError {
    status: ProviderStatus,
    message: String,
}

impl ZaiConfigError {
    fn auth(message: impl Into<String>) -> Self {
        Self {
            status: ProviderStatus::AuthRequired,
            message: message.into(),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: ProviderStatus::Unavailable,
            message: message.into(),
        }
    }
}

struct ZaiConfig {
    api_key: String,
    quota_url: String,
    source_label: String,
    organization_id: Option<String>,
    project_id: Option<String>,
}

fn resolve_config(allow_custom_endpoints: bool) -> Result<ZaiConfig, ZaiConfigError> {
    let api_key = read_env_trim(&["ZAI_API_KEY", "Z_AI_API_KEY", "ZHIPU_API_KEY"])
        .or_else(|| read_home_file(".api-zai").map(|s| s.trim().to_string()))
        .or_else(read_opencode_zai_key)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            ZaiConfigError::auth(
                "No API key found. Set ZAI_API_KEY, Z_AI_API_KEY, ~/.api-zai, or ~/.local/share/opencode/auth.json",
            )
        })?;

    let organization_id = read_env_trim(&[
        "Z_AI_BIGMODEL_ORGANIZATION",
        "Z_AI_ORGANIZATION_ID",
        "ZAI_ORGANIZATION_ID",
        "BIGMODEL_ORGANIZATION",
        "BIGMODEL_ORGANIZATION_ID",
    ]);
    let project_id = read_env_trim(&[
        "Z_AI_BIGMODEL_PROJECT",
        "Z_AI_PROJECT_ID",
        "Z_AI_WORKSPACE_ID",
        "ZAI_PROJECT_ID",
        "BIGMODEL_PROJECT",
        "BIGMODEL_PROJECT_ID",
    ]);
    let team_mode = organization_id.is_some() || project_id.is_some();
    let quota_url =
        resolve_quota_url(team_mode, allow_custom_endpoints).map_err(ZaiConfigError::invalid)?;
    let source_label = reqwest::Url::parse(&quota_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| DEFAULT_ZAI_HOST.to_string());

    Ok(ZaiConfig {
        api_key,
        quota_url,
        source_label,
        organization_id,
        project_id,
    })
}

fn resolve_quota_url(team_mode: bool, allow_custom_endpoints: bool) -> Result<String, String> {
    if let Some(url) = read_env_trim(&["Z_AI_QUOTA_URL", "ZAI_QUOTA_URL"]) {
        let mut normalized = normalize_https_url(&url, allow_custom_endpoints)?;
        append_team_query(&mut normalized, team_mode);
        return Ok(normalized);
    }

    let host = read_env_trim(&["Z_AI_API_HOST", "ZAI_API_HOST"])
        .unwrap_or_else(|| DEFAULT_ZAI_HOST.to_string());
    let host = normalize_host(&host, allow_custom_endpoints)?;
    let mut url = format!("https://{}{}", host.trim_end_matches('/'), DEFAULT_ZAI_PATH);
    append_team_query(&mut url, team_mode);
    Ok(url)
}

fn append_team_query(url: &mut String, team_mode: bool) {
    if !team_mode {
        return;
    }
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return;
    };
    let has_team_type = parsed
        .query_pairs()
        .any(|(key, value)| key == "type" && value == "2");
    if !has_team_type {
        parsed.query_pairs_mut().append_pair("type", "2");
        *url = parsed.to_string();
    }
}

fn normalize_https_url(input: &str, allow_custom_endpoints: bool) -> Result<String, String> {
    let trimmed = input.trim();
    let candidate = if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    };
    let url =
        reqwest::Url::parse(&candidate).map_err(|_| "Invalid Z.ai API endpoint".to_string())?;
    validate_endpoint(&url, allow_custom_endpoints)?;
    Ok(url.to_string())
}

fn normalize_host(input: &str, allow_custom_endpoints: bool) -> Result<String, String> {
    let normalized =
        normalize_https_url(input.trim().trim_end_matches('/'), allow_custom_endpoints)?;
    let url = reqwest::Url::parse(&normalized).map_err(|_| "Invalid Z.ai API host".to_string())?;
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err("Z.ai API host must not include a path, query, or fragment".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "Z.ai API host is missing a hostname".to_string())?;
    Ok(match url.port() {
        Some(port) => format!("{}:{}", host, port),
        None => host.to_string(),
    })
}

fn validate_endpoint(url: &reqwest::Url, allow_custom_endpoints: bool) -> Result<(), String> {
    if url.scheme() != "https" {
        return Err("Refusing to send Z.ai API key to a non-HTTPS endpoint".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Z.ai API endpoint must not contain credentials".to_string());
    }
    if url.fragment().is_some() {
        return Err("Z.ai API endpoint must not contain a URL fragment".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "Z.ai API endpoint is missing a hostname".to_string())?;
    if !allow_custom_endpoints && !is_official_zai_host(host) {
        return Err(format!(
            "Refusing to send Z.ai API key to {host}. Use --allow-custom-endpoints to permit non-official endpoints"
        ));
    }
    Ok(())
}

fn is_official_zai_host(host: &str) -> bool {
    ["z.ai", "bigmodel.cn", "zhipuai.cn"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn read_env_trim(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn first_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn read_opencode_zai_key() -> Option<String> {
    let content = read_home_file(".local/share/opencode/auth.json")?;
    let auth: serde_json::Value = serde_json::from_str(&content).ok()?;
    auth.get("zai-coding-plan")
        .and_then(|e| e.get("key"))
        .and_then(|k| k.as_str())
        .map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_real_remaining_from_usage_and_current_value() {
        let limit: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/zai/usage-current-value.json"
        ))
        .unwrap();

        let window = zai_window_from_limit(&limit).unwrap();
        assert_eq!(window.used_percent, Some(10.0));
        assert_eq!(window.limit, 1000);
        assert_eq!(window.remaining, 900);
        assert!(window.has_real_remaining);
    }

    #[test]
    fn missing_usage_and_percentage_stays_unknown() {
        let limit = serde_json::json!({
            "type": "TOKENS_LIMIT",
            "unit": 3,
            "number": 5
        });

        let window = zai_window_from_limit(&limit).unwrap();
        assert_eq!(window.used_percent, None);
        assert_eq!(window.limit, 0);
        assert_eq!(window.remaining, 0);
        assert!(!window.has_real_remaining);
    }

    #[test]
    fn real_remaining_units_are_clamped_to_the_limit() {
        let limit = serde_json::json!({
            "type": "TOKENS_LIMIT",
            "unit": 3,
            "number": 5,
            "usage": 100,
            "currentValue": 0,
            "remaining": 150
        });

        let window = zai_window_from_limit(&limit).unwrap();
        assert_eq!(window.remaining, 100);
        assert_eq!(window.used_percent, Some(0.0));
    }

    #[test]
    fn team_query_uses_an_exact_query_pair() {
        let mut url = "https://api.z.ai/quota?prototype=2".to_string();
        append_team_query(&mut url, true);
        let parsed = reqwest::Url::parse(&url).unwrap();
        assert!(
            parsed
                .query_pairs()
                .any(|(key, value)| key == "type" && value == "2")
        );
    }

    #[test]
    fn accepts_official_zai_endpoints_by_default() {
        assert!(normalize_https_url("https://api.z.ai/quota", false).is_ok());
        assert!(normalize_https_url("open.bigmodel.cn/quota", false).is_ok());
    }

    #[test]
    fn rejects_lookalike_and_custom_hosts_by_default() {
        assert!(normalize_https_url("https://z.ai.example.com/quota", false).is_err());
        assert!(normalize_https_url("https://example.com/quota", false).is_err());
    }

    #[test]
    fn accepts_custom_https_endpoints_only_with_opt_in() {
        assert!(normalize_https_url("https://example.com/quota", true).is_ok());
        assert!(normalize_https_url("http://example.com/quota", true).is_err());
    }
}
