use crate::engine::FetchOptions;
use crate::engine::snapshot::{
    FetchSource, ProviderId, ProviderResult, ProviderStatus, QuotaWindow, WindowType,
};
use crate::utils::{home_dir, network_error_message, read_home_file};
use std::sync::Arc;
use std::time::Duration;

const MINIMAX_NAME: &str = "MiniMax";
const MINIMAX_PLAN: &str = "Token Plan";
const DEFAULT_MINIMAX_HOST: &str = "platform.minimax.io";
const TOKEN_REMAINS_PATH: &str = "/v1/token_plan/remains";
const WEB_REMAINS_PATH: &str = "/v1/api/openplatform/coding_plan/remains";
const MINIMAX_COOKIE_QUERY: &str = "
    SELECT name, value FROM moz_cookies
    WHERE (
        host = 'minimax.io' OR host LIKE '%.minimax.io'
        OR host = 'minimaxi.com' OR host LIKE '%.minimaxi.com'
    )
    AND (name LIKE '%session%' OR name LIKE '%token%' OR name LIKE '%auth%')
    ORDER BY lastAccessed DESC
    LIMIT 1";

pub async fn fetch(options: &FetchOptions) -> ProviderResult {
    let id = ProviderId::MiniMax;
    let timeout = &options.timeout;
    let (api_key_from_file, plan_from_file) = read_minimax_file();
    let client = reqwest::Client::new();

    let mut retry = None;
    let mut had_api_key = false;
    if let Some(api_key) = resolve_api_key(api_key_from_file) {
        had_api_key = true;
        match fetch_with_api_key(
            &client,
            &api_key,
            plan_from_file.as_deref(),
            timeout,
            options.allow_custom_endpoints,
        )
        .await
        {
            FetchOutcome::Success(result) => return result,
            FetchOutcome::RetryWeb {
                message,
                auth_required,
            } => retry = Some((message, auth_required)),
            FetchOutcome::Terminal(result) => return result,
        }
    }

    if let Some(cookie) = resolve_cookie_header(options.allow_browser_cookies) {
        return fetch_with_cookie(
            &client,
            cookie,
            plan_from_file.as_deref(),
            timeout,
            options.allow_custom_endpoints,
        )
        .await;
    }

    if had_api_key {
        let (message, auth_required) = retry.unwrap_or_else(|| {
            (
                "MiniMax API failed and no web cookie fallback is configured".to_string(),
                false,
            )
        });
        return if auth_required {
            auth_result(
                plan_from_file.as_deref(),
                message,
                FetchSource::ApiKey,
                None,
            )
        } else {
            error_result(
                id,
                plan_from_file.as_deref(),
                message,
                FetchSource::ApiKey,
                None,
            )
        };
    }

    ProviderResult {
        provider: id,
        name: MINIMAX_NAME.to_string(),
        status: ProviderStatus::AuthRequired,
        error: Some("No MiniMax coding API key or configured web cookie found. Sign in to MiniMax in Firefox without --no-browser-cookies, or set MINIMAX_CODING_API_KEY, MINIMAX_API_KEY, ~/.api-minimax, MINIMAX_COOKIE_HEADER, or MINIMAX_COOKIE".into()),
        plan: Some(resolve_plan_label(plan_from_file.as_deref())),
        source: Some(FetchSource::ApiKey),
        source_label: None,
        windows: vec![],
        credits: vec![],
        metrics: vec![],
        fetched_at: chrono::Local::now(),
    }
}

enum FetchOutcome {
    Success(ProviderResult),
    RetryWeb {
        message: String,
        auth_required: bool,
    },
    Terminal(ProviderResult),
}

async fn fetch_with_api_key(
    client: &reqwest::Client,
    api_key: &str,
    file_plan: Option<&str>,
    timeout: &Arc<Duration>,
    allow_custom_endpoints: bool,
) -> FetchOutcome {
    let id = ProviderId::MiniMax;
    if api_key.trim().is_empty() {
        return FetchOutcome::RetryWeb {
            message: "MiniMax API key is empty".to_string(),
            auth_required: true,
        };
    }

    let endpoint_groups = match resolve_api_endpoint_groups(allow_custom_endpoints) {
        Ok(groups) => groups,
        Err(message) => {
            return FetchOutcome::Terminal(error_result(
                id,
                file_plan,
                message,
                FetchSource::ApiKey,
                None,
            ));
        }
    };

    let mut last_message = "MiniMax API endpoints returned no usable quota data".to_string();
    let mut saw_credential_failure = false;
    for (group_index, endpoints) in endpoint_groups.into_iter().enumerate() {
        let mut credential_failure = false;
        for endpoint in endpoints {
            let source_label = endpoint_label(&endpoint);
            let response = client
                .get(&endpoint)
                .header("Authorization", format!("Bearer {}", api_key.trim()))
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .header("MM-API-Source", "remaining")
                .timeout(*timeout.as_ref())
                .send()
                .await;

            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    last_message = format!(
                        "MiniMax API request failed: {}",
                        network_error_message(&error)
                    );
                    continue;
                }
            };
            let status = response.status();
            let body_text = match response.text().await {
                Ok(body) => body,
                Err(error) => {
                    last_message = format!(
                        "MiniMax API response failed: {}",
                        network_error_message(&error)
                    );
                    continue;
                }
            };

            if status.as_u16() == 401 || status.as_u16() == 403 {
                credential_failure = true;
                saw_credential_failure = true;
                last_message = format!(
                    "MiniMax API authentication failed at {} (HTTP {})",
                    source_label.as_deref().unwrap_or("configured endpoint"),
                    status.as_u16()
                );
                continue;
            }
            if status.as_u16() == 404 || status.as_u16() == 405 {
                last_message = format!(
                    "MiniMax API endpoint is unavailable at {} (HTTP {})",
                    source_label.as_deref().unwrap_or("configured endpoint"),
                    status.as_u16()
                );
                continue;
            }
            if !status.is_success() {
                return FetchOutcome::Terminal(error_result(
                    id,
                    file_plan,
                    format!("MiniMax API returned HTTP {}", status.as_u16()),
                    FetchSource::ApiKey,
                    source_label,
                ));
            }

            match parse_response(&body_text, file_plan, FetchSource::ApiKey, source_label) {
                Ok(result) => return FetchOutcome::Success(result),
                Err(message) => last_message = message,
            }
        }

        // Only cross from the global API to the China API when credentials were rejected.
        if group_index == 0 && !credential_failure {
            break;
        }
    }

    FetchOutcome::RetryWeb {
        message: last_message,
        auth_required: saw_credential_failure,
    }
}

async fn fetch_with_cookie(
    client: &reqwest::Client,
    cookie: CookieSource,
    file_plan: Option<&str>,
    timeout: &Arc<Duration>,
    allow_custom_endpoints: bool,
) -> ProviderResult {
    let endpoint = match resolve_endpoint(
        &["MINIMAX_WEB_REMAINS_URL", "MINIMAX_REMAINS_URL"],
        WEB_REMAINS_PATH,
        allow_custom_endpoints,
    ) {
        Ok(url) => url,
        Err(message) => {
            return error_result(
                ProviderId::MiniMax,
                file_plan,
                message,
                FetchSource::BrowserCookie,
                Some(cookie.source_label),
            );
        }
    };

    let resp = client
        .get(&endpoint)
        .header("Cookie", cookie.header)
        .header("Accept", "application/json")
        .header("User-Agent", "Mozilla/5.0")
        .timeout(*timeout.as_ref())
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            return network_result(
                ProviderId::MiniMax,
                file_plan,
                network_error_message(&e),
                FetchSource::BrowserCookie,
                Some(cookie.source_label),
            );
        }
    };

    let status = resp.status();
    let body_text = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            return error_result(
                ProviderId::MiniMax,
                file_plan,
                network_error_message(&e),
                FetchSource::BrowserCookie,
                Some(cookie.source_label),
            );
        }
    };

    if status.as_u16() == 401 || status.as_u16() == 403 {
        return auth_result(
            file_plan,
            format!(
                "MiniMax web session failed (HTTP {}). Refresh cookie.",
                status.as_u16()
            ),
            FetchSource::BrowserCookie,
            Some(cookie.source_label),
        );
    }
    if !status.is_success() {
        return error_result(
            ProviderId::MiniMax,
            file_plan,
            format!("MiniMax web endpoint returned HTTP {}", status.as_u16()),
            FetchSource::BrowserCookie,
            Some(cookie.source_label),
        );
    }

    match parse_response(
        &body_text,
        file_plan,
        FetchSource::BrowserCookie,
        Some(cookie.source_label),
    ) {
        Ok(result) => result,
        Err(message) => error_result(
            ProviderId::MiniMax,
            file_plan,
            message,
            FetchSource::BrowserCookie,
            None,
        ),
    }
}

fn parse_response(
    body_text: &str,
    file_plan: Option<&str>,
    source: FetchSource,
    source_label: Option<String>,
) -> Result<ProviderResult, String> {
    let body: serde_json::Value =
        serde_json::from_str(body_text).map_err(|e| format!("Invalid JSON from MiniMax: {}", e))?;

    let payload = body.get("data").unwrap_or(&body);
    let base_resp = payload
        .get("base_resp")
        .or_else(|| payload.get("baseResp"))
        .or_else(|| body.get("base_resp"))
        .or_else(|| body.get("baseResp"));
    let status_code = base_resp
        .and_then(|b| b.get("status_code").or_else(|| b.get("statusCode")))
        .and_then(value_to_i64)
        .unwrap_or(0);
    let status_msg = base_resp
        .and_then(|b| b.get("status_msg").or_else(|| b.get("statusMsg")))
        .and_then(|m| m.as_str())
        .unwrap_or("");

    if status_code != 0 {
        return Err(format!(
            "MiniMax API error: {} ({})",
            status_msg, status_code
        ));
    }

    let model_remains = payload
        .get("model_remains")
        .or_else(|| payload.get("modelRemains"))
        .and_then(|m| m.as_array())
        .ok_or_else(|| "No model_remains data in response".to_string())?;

    let mut windows = Vec::new();
    for (index, window) in model_remains.iter().enumerate() {
        let name = window
            .get("model_name")
            .or_else(|| window.get("modelName"))
            .and_then(|n| n.as_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(if index == 0 { "general" } else { "unknown" });
        let interval_label = if name.eq_ignore_ascii_case("general") {
            "5h".to_string()
        } else {
            format!("5h ({})", name)
        };
        push_window(
            &mut windows,
            window,
            WindowFieldKeys {
                remaining_percent: &[
                    "current_interval_remaining_percent",
                    "currentIntervalRemainingPercent",
                ],
                total: &["current_interval_total_count", "currentIntervalTotalCount"],
                remaining_count: &["current_interval_usage_count", "currentIntervalUsageCount"],
                end_time: &["end_time", "endTime"],
                remains_time: &["remains_time", "remainsTime"],
            },
            &interval_label,
            WindowType::FiveHours,
        );

        if is_text_generation_model(name) {
            let weekly_label = if name.eq_ignore_ascii_case("general") {
                "Weekly".to_string()
            } else {
                format!("Weekly ({})", name)
            };
            push_window(
                &mut windows,
                window,
                WindowFieldKeys {
                    remaining_percent: &[
                        "current_weekly_remaining_percent",
                        "currentWeeklyRemainingPercent",
                    ],
                    total: &["current_weekly_total_count", "currentWeeklyTotalCount"],
                    remaining_count: &["current_weekly_usage_count", "currentWeeklyUsageCount"],
                    end_time: &["weekly_end_time", "weeklyEndTime"],
                    remains_time: &["weekly_remains_time", "weeklyRemainsTime"],
                },
                &weekly_label,
                WindowType::Weekly,
            );
        }
    }

    if windows.is_empty() {
        return Err("No usable quota data in model_remains".into());
    }

    Ok(ProviderResult {
        provider: ProviderId::MiniMax,
        name: MINIMAX_NAME.to_string(),
        status: ProviderStatus::Available,
        error: None,
        plan: Some(response_plan_label(payload, file_plan)),
        source: Some(source),
        source_label,
        windows,
        credits: vec![],
        metrics: vec![],
        fetched_at: chrono::Local::now(),
    })
}

fn is_text_generation_model(model_name: &str) -> bool {
    let normalized = model_name.trim().to_ascii_lowercase();
    normalized == "general" || normalized.contains("minimax-m") || normalized.starts_with("m2.")
}

fn resolve_api_key(api_key_from_file: Option<String>) -> Option<String> {
    read_env_trim(&[
        "MINIMAX_CODING_API_KEY",
        "MINIMAX_CODING_PLAN_API_KEY",
        "MINIMAX_API_KEY",
        "MINIMAX_SUBSCRIPTION_KEY",
        "MINIMAX_TOKEN",
    ])
    .or(api_key_from_file)
    .or_else(read_opencode_minimax_key)
    .filter(|key| !key.trim().is_empty())
}

struct CookieSource {
    header: String,
    source_label: String,
}

fn resolve_cookie_header(allow_browser_cookies: bool) -> Option<CookieSource> {
    if let Some(raw) = read_env_trim(&["MINIMAX_COOKIE_HEADER", "MINIMAX_COOKIE"]) {
        return Some(CookieSource {
            header: normalize_cookie_header(&raw),
            source_label: "manual cookie header".to_string(),
        });
    }
    allow_browser_cookies.then(find_firefox_cookie).flatten()
}

fn find_firefox_cookie() -> Option<CookieSource> {
    let home = home_dir()?;
    let search_dirs = [
        home.join(".mozilla/firefox"),
        home.join("snap/firefox/common/.mozilla/firefox"),
    ];

    for profiles_dir in &search_dirs {
        if let Ok(entries) = std::fs::read_dir(profiles_dir) {
            for entry in entries.flatten() {
                let cookies_db = entry.path().join("cookies.sqlite");
                if !cookies_db.exists() {
                    continue;
                }
                let Ok(conn) = rusqlite::Connection::open_with_flags(
                    &cookies_db,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                ) else {
                    continue;
                };
                let result: Result<(String, String), _> =
                    conn.query_row(MINIMAX_COOKIE_QUERY, [], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    });
                if let Ok((name, value)) = result {
                    return Some(CookieSource {
                        header: format!("{}={}", name, value),
                        source_label: format!("Firefox cookie ({})", name),
                    });
                }
            }
        }
    }

    None
}

fn normalize_cookie_header(raw: &str) -> String {
    raw.trim().trim_start_matches("Cookie:").trim().to_string()
}

fn resolve_endpoint(
    env_keys: &[&str],
    default_path: &str,
    allow_custom_endpoints: bool,
) -> Result<String, String> {
    if let Some(url) = read_env_trim(env_keys) {
        return validate_https_url(&url, allow_custom_endpoints);
    }

    let host = read_env_trim(&["MINIMAX_HOST"])
        .unwrap_or_else(|| DEFAULT_MINIMAX_HOST.to_string())
        .trim()
        .trim_end_matches('/')
        .trim_start_matches("https://")
        .to_string();
    let url = format!("https://{}{}", host, default_path);
    validate_https_url(&url, allow_custom_endpoints)
}

fn resolve_api_endpoint_groups(allow_custom_endpoints: bool) -> Result<Vec<Vec<String>>, String> {
    if let Some(url) = read_env_trim(&["MINIMAX_CODING_PLAN_URL", "MINIMAX_REMAINS_URL"]) {
        return Ok(vec![vec![validate_https_url(
            &url,
            allow_custom_endpoints,
        )?]]);
    }

    if let Some(host) = read_env_trim(&["MINIMAX_HOST"]) {
        return Ok(vec![api_endpoints_for_host(&host, allow_custom_endpoints)?]);
    }

    Ok(vec![
        api_endpoints_for_host("api.minimax.io", false)?,
        api_endpoints_for_host("api.minimaxi.com", false)?,
    ])
}

fn api_endpoints_for_host(host: &str, allow_custom_endpoints: bool) -> Result<Vec<String>, String> {
    let base = if host.trim().starts_with("https://") {
        host.trim().to_string()
    } else {
        format!("https://{}", host.trim())
    };
    let validated = validate_https_url(&base, allow_custom_endpoints)?;
    let parsed = reqwest::Url::parse(&validated)
        .map_err(|_| "MiniMax API host is not a valid URL".to_string())?;
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("MiniMax API host must not include a path, query, or fragment".to_string());
    }
    let mut token = parsed.clone();
    token.set_path(TOKEN_REMAINS_PATH);
    token.set_query(None);
    let mut legacy = parsed;
    legacy.set_path(WEB_REMAINS_PATH);
    legacy.set_query(None);
    Ok(vec![token.to_string(), legacy.to_string()])
}

fn validate_https_url(raw: &str, allow_custom_endpoints: bool) -> Result<String, String> {
    let url = raw.trim();
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| "MiniMax endpoint override is not a valid URL".to_string())?;
    if parsed.scheme() != "https" {
        return Err("Refusing to send MiniMax credentials to a non-HTTPS endpoint".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Refusing MiniMax endpoint override with unsafe host syntax".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "MiniMax endpoint override has no host".to_string())?
        .to_ascii_lowercase();
    if !allow_custom_endpoints && !is_official_minimax_host(&host) {
        return Err(format!(
            "Refusing to send MiniMax credentials to {host}. Use --allow-custom-endpoints to permit non-official endpoints"
        ));
    }
    Ok(parsed.to_string())
}

fn endpoint_label(raw: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(raw).ok()?;
    let host = parsed.host_str()?;
    Some(match parsed.port() {
        Some(port) => format!("{}:{}", host, port),
        None => host.to_string(),
    })
}

fn is_official_minimax_host(host: &str) -> bool {
    host == "minimax.io"
        || host.ends_with(".minimax.io")
        || host == "minimaxi.com"
        || host.ends_with(".minimaxi.com")
}

fn read_env_trim(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn resolve_plan_label(file_plan: Option<&str>) -> String {
    let tier = std::env::var("MINIMAX_PLAN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            file_plan
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    match tier {
        Some(t) => format!("{} ({})", MINIMAX_PLAN, t),
        None => MINIMAX_PLAN.to_string(),
    }
}

fn read_minimax_file() -> (Option<String>, Option<String>) {
    let Some(content) = read_home_file(".api-minimax") else {
        return (None, None);
    };
    let mut lines = content.lines();
    let key = lines
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let plan = lines
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    (key, plan)
}

fn read_opencode_minimax_key() -> Option<String> {
    let content = read_home_file(".local/share/opencode/auth.json")?;
    let auth: serde_json::Value = serde_json::from_str(&content).ok()?;
    auth.get("minimax")
        .and_then(|entry| entry.get("key"))
        .and_then(|key| key.as_str())
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

fn response_plan_label(payload: &serde_json::Value, file_plan: Option<&str>) -> String {
    let plan = first_string(
        payload,
        &[
            "current_subscribe_title",
            "currentSubscribeTitle",
            "plan_name",
            "planName",
            "combo_title",
            "comboTitle",
            "current_plan_title",
            "currentPlanTitle",
        ],
    )
    .or_else(|| {
        payload
            .get("current_combo_card")
            .or_else(|| payload.get("currentComboCard"))
            .and_then(|card| first_string(card, &["title"]))
    });

    plan.unwrap_or_else(|| resolve_plan_label(file_plan))
}

struct WindowFieldKeys<'a> {
    remaining_percent: &'a [&'a str],
    total: &'a [&'a str],
    remaining_count: &'a [&'a str],
    end_time: &'a [&'a str],
    remains_time: &'a [&'a str],
}

fn push_window(
    windows: &mut Vec<QuotaWindow>,
    entry: &serde_json::Value,
    keys: WindowFieldKeys<'_>,
    label: &str,
    wtype: WindowType,
) {
    let reset_epoch = first_i64(entry, keys.end_time)
        .filter(|e| *e > 0)
        .map(|ms| if ms > 10_000_000_000 { ms / 1000 } else { ms })
        .or_else(|| {
            first_i64(entry, keys.remains_time)
                .filter(|secs| *secs > 0)
                .map(|raw| {
                    chrono::Utc::now().timestamp() + if raw > 1_000_000 { raw / 1000 } else { raw }
                })
        });

    if let Some(remaining_pct) = first_f64(entry, keys.remaining_percent) {
        let remaining_pct = remaining_pct.clamp(0.0, 100.0);
        let used_pct = (100.0 - remaining_pct).clamp(0.0, 100.0);
        windows.push(QuotaWindow {
            wtype,
            label: label.to_string(),
            used_percent: Some(used_pct),
            limit: 100,
            remaining: remaining_pct.round() as i64,
            resets_at_epoch: reset_epoch,
            resets_at_str: None,
            has_real_remaining: false,
            is_synthetic: false,
        });
        return;
    }

    let Some(total) = first_i64(entry, keys.total).filter(|total| *total > 0) else {
        return;
    };
    let Some(remaining) = first_i64(entry, keys.remaining_count) else {
        return;
    };
    let remaining = remaining.clamp(0, total);
    let used = total - remaining;
    let used_pct = (used as f64 * 100.0 / total as f64).clamp(0.0, 100.0);

    windows.push(QuotaWindow {
        wtype,
        label: label.to_string(),
        used_percent: Some(used_pct),
        limit: total,
        remaining,
        resets_at_epoch: reset_epoch,
        resets_at_str: None,
        has_real_remaining: true,
        is_synthetic: false,
    });
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

fn first_i64(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(value_to_i64))
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

fn value_to_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
        .or_else(|| value.as_f64().map(|f| f.round() as i64))
        .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
}

fn auth_result(
    file_plan: Option<&str>,
    message: String,
    source: FetchSource,
    source_label: Option<String>,
) -> ProviderResult {
    ProviderResult {
        provider: ProviderId::MiniMax,
        name: MINIMAX_NAME.to_string(),
        status: ProviderStatus::AuthRequired,
        error: Some(message),
        plan: Some(resolve_plan_label(file_plan)),
        source: Some(source),
        source_label,
        windows: vec![],
        credits: vec![],
        metrics: vec![],
        fetched_at: chrono::Local::now(),
    }
}

fn network_result(
    id: ProviderId,
    file_plan: Option<&str>,
    message: String,
    source: FetchSource,
    source_label: Option<String>,
) -> ProviderResult {
    ProviderResult {
        provider: id,
        name: MINIMAX_NAME.to_string(),
        status: ProviderStatus::NetworkError,
        error: Some(message),
        plan: Some(resolve_plan_label(file_plan)),
        source: Some(source),
        source_label,
        windows: vec![],
        credits: vec![],
        metrics: vec![],
        fetched_at: chrono::Local::now(),
    }
}

fn error_result(
    id: ProviderId,
    file_plan: Option<&str>,
    message: String,
    source: FetchSource,
    source_label: Option<String>,
) -> ProviderResult {
    ProviderResult {
        provider: id,
        name: MINIMAX_NAME.to_string(),
        status: ProviderStatus::Unavailable,
        error: Some(message),
        plan: Some(resolve_plan_label(file_plan)),
        source: Some(source),
        source_label,
        windows: vec![],
        credits: vec![],
        metrics: vec![],
        fetched_at: chrono::Local::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_non_general_model_is_not_duplicated() {
        let response = serde_json::json!({
            "base_resp": {"status_code": 0},
            "model_remains": [{
                "model_name": "M2.7-highspeed",
                "current_interval_total_count": 1000,
                "current_interval_usage_count": 250,
                "current_weekly_total_count": 6000,
                "current_weekly_usage_count": 5000
            }]
        });

        let result = parse_response(
            &response.to_string(),
            None,
            FetchSource::ApiKey,
            Some("fixture".to_string()),
        )
        .unwrap();

        assert_eq!(result.windows.len(), 2);
        assert_eq!(result.windows[0].used_percent, Some(75.0));
        assert!((result.windows[1].used_percent.unwrap() - 16.666_666).abs() < 0.001);
    }

    #[test]
    fn provider_host_check_rejects_lookalike_suffix() {
        assert!(!is_official_minimax_host("evilminimax.io"));
        assert!(is_official_minimax_host("api.minimax.io"));
    }

    #[test]
    fn cookie_query_rejects_lookalike_domains() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_cookies (
                host TEXT NOT NULL,
                name TEXT NOT NULL,
                value TEXT NOT NULL,
                lastAccessed INTEGER NOT NULL
            );
            INSERT INTO moz_cookies VALUES
                ('evilminimax.io', 'session', 'evil', 20),
                ('.minimax.io', 'session', 'official', 10);",
        )
        .unwrap();

        let value: String = conn
            .query_row(MINIMAX_COOKIE_QUERY, [], |row| row.get(1))
            .unwrap();
        assert_eq!(value, "official");
    }

    #[test]
    fn custom_endpoints_require_explicit_opt_in() {
        assert!(validate_https_url("https://api.minimax.io/remains", false).is_ok());
        assert!(validate_https_url("https://example.com/remains", false).is_err());
        assert!(validate_https_url("https://example.com/remains", true).is_ok());
        assert!(validate_https_url("http://example.com/remains", true).is_err());
    }

    #[test]
    fn endpoint_labels_do_not_expose_paths_or_queries() {
        assert_eq!(
            endpoint_label("https://example.com/private?token=secret"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn parses_token_plan_fixture() {
        let result = parse_response(
            include_str!("../../tests/fixtures/minimax/token-plan-normal.json"),
            None,
            FetchSource::ApiKey,
            Some("fixture".to_string()),
        )
        .unwrap();

        assert_eq!(result.plan.as_deref(), Some("Token Plan Plus"));
        assert_eq!(result.windows.len(), 2);
        assert_eq!(result.windows[0].used_percent, Some(4.0));
        assert_eq!(result.windows[1].used_percent, Some(1.0));
    }
}
