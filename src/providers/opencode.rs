use crate::engine::FetchOptions;
use crate::engine::snapshot::{
    FetchSource, ProviderId, ProviderResult, ProviderStatus, QuotaUnit, QuotaWindow, UsageMetric,
    WindowType,
};
use crate::utils::{home_dir, network_error_message};
use aes::cipher::{BlockDecryptMut, KeyIvInit};
use chrono::{Datelike, TimeZone};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

const OPENCODE_NAME: &str = "OpenCode";
const OPENCODE_GO_PLAN: &str = "Go plan";
const OPENCODE_SERVER_URL: &str = "https://opencode.ai/_server";
const OPENCODE_WORKSPACES_FN: &str =
    "def39973159c7f0483d8793a822b8dbb10d067e12c65455fcb4608459ba0234f";

pub async fn fetch(options: &FetchOptions) -> ProviderResult {
    let timeout = &options.timeout;
    let id = ProviderId::OpenCode;
    let client = reqwest::Client::new();
    let config = resolve_web_config(options.allow_browser_cookies);
    let mut last_error: Option<String> = None;

    if let Some(config) = config {
        if let Some(cookie) = config.cookie_header.as_deref() {
            let workspace_id = match config.workspace_id.clone() {
                Some(workspace) => Some(workspace),
                None => match fetch_workspace_id_from_server(&client, cookie, timeout).await {
                    Some(workspace) => Some(workspace),
                    None => {
                        last_error =
                            Some("Could not resolve OpenCode workspace from _server".to_string());
                        None
                    }
                },
            };

            if let Some(workspace_id) = workspace_id {
                match fetch_workspace_page(&client, cookie, &workspace_id, timeout).await {
                    Ok(html) => {
                        let result = parse_usage_data(
                            &html,
                            id,
                            FetchSource::WebScrape,
                            Some(format!("workspace page ({})", config.source_label)),
                        );
                        if result.status == ProviderStatus::Available {
                            return result;
                        }
                        last_error = result.error.clone();
                    }
                    Err(message) => last_error = Some(message),
                }
            }
        } else {
            last_error = Some("OpenCode web credentials are missing an auth cookie".to_string());
        }
    }

    if let Some(result) = local_database_result(id) {
        return result;
    }

    ProviderResult {
        provider: id,
        name: OPENCODE_NAME.to_string(),
        status: ProviderStatus::AuthRequired,
        error: Some(last_error.unwrap_or_else(|| {
            "Configure OpenCode Go: set OPENCODE_GO_WORKSPACE_ID + OPENCODE_GO_AUTH_COOKIE, create ~/.config/opencode-quota/opencode-go.json, or keep ~/.local/share/opencode/opencode.db available".into()
        })),
        plan: Some(OPENCODE_GO_PLAN.to_string()),
        source: Some(FetchSource::WebScrape),
        source_label: None,
        windows: vec![],
        credits: vec![],
        metrics: vec![],
        fetched_at: chrono::Local::now(),
    }
}

async fn fetch_workspace_page(
    client: &reqwest::Client,
    cookie: &str,
    workspace_id: &str,
    timeout: &Arc<Duration>,
) -> Result<String, String> {
    let url = format!("https://opencode.ai/workspace/{}/go", workspace_id);
    let resp = client
        .get(&url)
        .header("Cookie", normalize_cookie_header(cookie, "auth"))
        .header(
            "User-Agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36",
        )
        .header("Accept", "text/html")
        .timeout(*timeout.as_ref())
        .send()
        .await
        .map_err(|error| network_error_message(&error))?;

    let status = resp.status();
    let html = resp
        .text()
        .await
        .map_err(|error| network_error_message(&error))?;
    if !status.is_success() {
        return Err(format!(
            "OpenCode workspace page returned HTTP {}",
            status.as_u16()
        ));
    }
    Ok(html)
}

async fn fetch_workspace_id_from_server(
    client: &reqwest::Client,
    cookie: &str,
    timeout: &Arc<Duration>,
) -> Option<String> {
    if let Ok(text) =
        server_function_request(client, OPENCODE_WORKSPACES_FN, cookie, "GET", None, timeout).await
    {
        if let Some(workspace) = extract_workspace_id(&text) {
            return Some(workspace);
        }
    }
    let text = server_function_request(
        client,
        OPENCODE_WORKSPACES_FN,
        cookie,
        "POST",
        Some("[]"),
        timeout,
    )
    .await
    .ok()?;
    extract_workspace_id(&text)
}

async fn server_function_request(
    client: &reqwest::Client,
    function_id: &str,
    cookie: &str,
    method: &str,
    args: Option<&str>,
    timeout: &Arc<Duration>,
) -> Result<String, String> {
    let mut url = reqwest::Url::parse(OPENCODE_SERVER_URL)
        .map_err(|error| format!("Invalid OpenCode server URL: {}", error))?;
    if method == "GET" {
        let mut query = url.query_pairs_mut();
        query.append_pair("id", function_id);
        if let Some(args) = args {
            query.append_pair("args", args);
        }
    }

    let mut request = client
        .request(
            if method == "GET" {
                reqwest::Method::GET
            } else {
                reqwest::Method::POST
            },
            url,
        )
        .header("Cookie", normalize_cookie_header(cookie, "auth"))
        .header("Accept", "text/javascript, application/json, */*")
        .header(
            "User-Agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36",
        )
        .header("X-Server-Id", function_id)
        .header(
            "X-Server-Instance",
            format!(
                "server-fn:{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
            ),
        )
        .header("Origin", "https://opencode.ai")
        .header("Referer", "https://opencode.ai/")
        .timeout(*timeout.as_ref());
    if method != "GET" {
        request = request
            .header("Content-Type", "application/json")
            .body(args.unwrap_or("[]").to_string());
    }
    let resp = request
        .send()
        .await
        .map_err(|error| network_error_message(&error))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|error| network_error_message(&error))?;
    if !status.is_success() {
        return Err(format!(
            "OpenCode _server returned HTTP {}",
            status.as_u16()
        ));
    }
    Ok(text)
}

struct WebConfig {
    workspace_id: Option<String>,
    cookie_header: Option<String>,
    source_label: String,
}

fn resolve_web_config(allow_browser_cookies: bool) -> Option<WebConfig> {
    let config = read_config_json();
    let workspace_id = read_env_trim(&["OPENCODE_GO_WORKSPACE_ID", "OPENCODE_WORKSPACE_ID"])
        .or_else(|| {
            config
                .as_ref()
                .and_then(|json| json.get("workspaceId").and_then(|value| value.as_str()))
                .map(str::to_string)
        })
        .and_then(|value| normalize_workspace_id(&value));

    let cookie_header = read_env_trim(&["OPENCODE_GO_AUTH_COOKIE", "OPENCODE_AUTH_COOKIE"])
        .or_else(|| {
            config
                .as_ref()
                .and_then(|json| json.get("authCookie").and_then(|value| value.as_str()))
                .map(str::to_string)
        })
        .or_else(|| {
            allow_browser_cookies
                .then(|| tokio::task::block_in_place(decrypt_chrome_cookie))
                .flatten()
        });

    if workspace_id.is_none() && cookie_header.is_none() {
        return None;
    }

    let source_label = if cookie_header.is_some() {
        "auth cookie".to_string()
    } else {
        "workspace config".to_string()
    };

    Some(WebConfig {
        workspace_id,
        cookie_header,
        source_label,
    })
}

fn read_config_json() -> Option<serde_json::Value> {
    let home = home_dir()?;
    let config_path = home.join(".config/opencode-quota/opencode-go.json");
    let content = std::fs::read_to_string(config_path).ok()?;
    serde_json::from_str(&content).ok()
}

fn read_env_trim(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn normalize_workspace_id(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(caps) = regex_lite::Regex::new(r"wrk_[A-Za-z0-9_\-]+")
        .ok()?
        .captures(text)
    {
        return caps.get(0).map(|m| m.as_str().to_string());
    }
    Some(text.trim_matches('/').to_string())
}

fn extract_workspace_id(text: &str) -> Option<String> {
    regex_lite::Regex::new(r"wrk_[A-Za-z0-9_\-]+")
        .ok()?
        .captures(text)
        .and_then(|caps| caps.get(0).map(|m| m.as_str().to_string()))
}

fn normalize_cookie_header(raw: &str, default_name: &str) -> String {
    let trimmed = raw.trim().trim_start_matches("Cookie:").trim();
    if trimmed.contains('=') {
        trimmed.to_string()
    } else {
        format!("{}={}", default_name, trimmed)
    }
}

fn decrypt_chrome_cookie() -> Option<String> {
    let home = home_dir()?;
    let cookie_db = home.join(".config/google-chrome/Default/Cookies");
    if !cookie_db.exists() {
        return None;
    }

    let temporary = copy_cookie_database(&cookie_db)?;

    let conn = rusqlite::Connection::open_with_flags(
        &temporary.0,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;

    let encrypted: Vec<u8> = conn
        .query_row(
            "SELECT encrypted_value FROM cookies WHERE host_key IN ('opencode.ai', '.opencode.ai') AND name='auth' ORDER BY expires_utc DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok()?;

    if encrypted.len() < 4 {
        return None;
    }

    let prefix = &encrypted[..3];
    if prefix != b"v11" && prefix != b"v10" {
        return None;
    }

    let ciphertext = &encrypted[3..];
    let key = get_chrome_safe_storage_key()?;
    let derived = derive_aes_key(&key);
    let iv = [0x20u8; 16];
    let decryptor = Aes128CbcDec::new_from_slices(&derived, &iv).ok()?;
    let mut buf = ciphertext.to_vec();
    let decrypted = decryptor.decrypt_padded_mut::<cbc::cipher::block_padding::Pkcs7>(&mut buf);

    match decrypted {
        Ok(plaintext) => String::from_utf8(plaintext.to_vec()).ok(),
        Err(_) => None,
    }
}

struct TemporaryCookieDatabase(PathBuf);

impl Drop for TemporaryCookieDatabase {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn copy_cookie_database(source: &Path) -> Option<TemporaryCookieDatabase> {
    let mut input = File::open(source).ok()?;
    for attempt in 0..16u32 {
        let filename = format!(
            "remaining-chrome-cookies-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            attempt
        );
        let path = std::env::temp_dir().join(filename);
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path);
        let Ok(mut output) = output else {
            continue;
        };
        if std::io::copy(&mut input, &mut output).is_ok() {
            return Some(TemporaryCookieDatabase(path));
        }
        let _ = std::fs::remove_file(path);
        return None;
    }
    None
}

fn get_chrome_safe_storage_key() -> Option<Vec<u8>> {
    let output = std::process::Command::new("secret-tool")
        .args(["lookup", "application", "chrome"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let key = String::from_utf8_lossy(&output.stdout);
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.as_bytes().to_vec())
}

fn derive_aes_key(password: &[u8]) -> [u8; 16] {
    use hmac::Hmac;
    use pbkdf2::pbkdf2;

    let mut key = [0u8; 16];
    let _ = pbkdf2::<Hmac<sha1::Sha1>>(password, b"saltysalt", 1, &mut key);
    key
}

fn local_database_result(id: ProviderId) -> Option<ProviderResult> {
    let path = local_database_path()?;
    let (windows, metrics) = local_database_usage(&path).ok()?;
    if windows.is_empty() {
        return None;
    }

    Some(ProviderResult {
        provider: id,
        name: OPENCODE_NAME.to_string(),
        status: ProviderStatus::Available,
        error: None,
        plan: Some(format!("{} local history", OPENCODE_GO_PLAN)),
        source: Some(FetchSource::LocalDatabase),
        source_label: Some("~/.local/share/opencode/opencode.db".to_string()),
        windows,
        credits: vec![],
        metrics,
        fetched_at: chrono::Local::now(),
    })
}

fn local_database_path() -> Option<PathBuf> {
    let home = home_dir()?;
    let path = home.join(".local/share/opencode/opencode.db");
    path.exists().then_some(path)
}

#[derive(Debug, Clone, Copy)]
struct LocalUsageRow {
    created_ms: i64,
    cost: f64,
}

fn local_database_usage(
    path: &Path,
) -> Result<(Vec<QuotaWindow>, Vec<UsageMetric>), rusqlite::Error> {
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let rows = read_local_usage_rows(&conn)?;
    if rows.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let now = chrono::Utc::now();
    let session_start_ms = now_ms - 5 * 60 * 60 * 1000;
    let week_start_date =
        now.date_naive() - chrono::Duration::days(i64::from(now.weekday().num_days_from_monday()));
    let week_start = chrono::Utc
        .from_utc_datetime(&week_start_date.and_hms_opt(0, 0, 0).unwrap())
        .timestamp_millis();
    let week_end = week_start + 7 * 24 * 60 * 60 * 1000;
    let month_start = chrono::Utc
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let (next_year, next_month) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    let month_end = chrono::Utc
        .with_ymd_and_hms(next_year, next_month, 1, 0, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();

    let sum = |start: i64, end: i64| {
        rows.iter()
            .filter(|row| row.created_ms >= start && row.created_ms < end)
            .map(|row| row.cost)
            .sum::<f64>()
    };
    let session_cost = sum(session_start_ms, now_ms);
    let weekly_cost = sum(week_start, week_end);
    let monthly_cost = sum(month_start, month_end);
    if session_cost <= 0.0 && weekly_cost <= 0.0 && monthly_cost <= 0.0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let rolling_reset = rows
        .iter()
        .filter(|row| row.created_ms >= session_start_ms && row.created_ms < now_ms)
        .map(|row| row.created_ms)
        .min()
        .unwrap_or(now_ms)
        + 5 * 60 * 60 * 1000;

    let windows = vec![
        local_cost_window(
            "5h",
            WindowType::FiveHours,
            session_cost,
            12.0,
            rolling_reset / 1000,
        ),
        local_cost_window(
            "Weekly",
            WindowType::Weekly,
            weekly_cost,
            30.0,
            week_end / 1000,
        ),
        local_cost_window(
            "Monthly",
            WindowType::Monthly,
            monthly_cost,
            60.0,
            month_end / 1000,
        ),
    ];
    let metrics = [
        ("5h", session_cost),
        ("weekly", weekly_cost),
        ("monthly", monthly_cost),
    ]
    .into_iter()
    .map(|(period, value)| UsageMetric {
        label: "cost".to_string(),
        value,
        unit: QuotaUnit::Usd,
        period: Some(period.to_string()),
        source_label: Some("opencode.db".to_string()),
    })
    .collect();
    Ok((windows, metrics))
}

fn local_cost_window(
    label: &str,
    wtype: WindowType,
    used_cost: f64,
    cost_limit: f64,
    resets_at_epoch: i64,
) -> QuotaWindow {
    let used_percent = (used_cost / cost_limit * 100.0).clamp(0.0, 100.0);
    QuotaWindow {
        wtype,
        label: label.to_string(),
        used_percent: Some(used_percent),
        limit: 100,
        remaining: (100.0 - used_percent).round() as i64,
        resets_at_epoch: Some(resets_at_epoch),
        resets_at_str: None,
        has_real_remaining: false,
        is_synthetic: true,
    }
}

fn read_local_usage_rows(
    conn: &rusqlite::Connection,
) -> Result<Vec<LocalUsageRow>, rusqlite::Error> {
    let has_part = sqlite_table_exists(conn, "part")?;
    let sql = if has_part {
        r#"
        WITH message_costs AS (
          SELECT
            id AS messageID,
            CAST(COALESCE(json_extract(data, '$.time.created'), time_created) AS INTEGER) AS createdMs,
            CAST(json_extract(data, '$.cost') AS REAL) AS cost
          FROM message
          WHERE json_valid(data)
            AND json_extract(data, '$.providerID') = 'opencode-go'
            AND json_extract(data, '$.role') = 'assistant'
            AND json_type(data, '$.cost') IN ('integer', 'real')
        )
        SELECT createdMs, cost FROM message_costs
        UNION ALL
        SELECT
          CAST(COALESCE(json_extract(p.data, '$.time.created'), p.time_created, m.time_created) AS INTEGER),
          CAST(json_extract(p.data, '$.cost') AS REAL)
        FROM part p
        JOIN message m ON m.id = p.message_id
        WHERE json_valid(p.data)
          AND json_valid(m.data)
          AND json_extract(p.data, '$.type') = 'step-finish'
          AND json_type(p.data, '$.cost') IN ('integer', 'real')
          AND json_extract(m.data, '$.providerID') = 'opencode-go'
          AND json_extract(m.data, '$.role') = 'assistant'
          AND NOT EXISTS (SELECT 1 FROM message_costs WHERE messageID = p.message_id)
        "#
    } else {
        r#"
        SELECT
          CAST(COALESCE(json_extract(data, '$.time.created'), time_created) AS INTEGER),
          CAST(json_extract(data, '$.cost') AS REAL)
        FROM message
        WHERE json_valid(data)
          AND json_extract(data, '$.providerID') = 'opencode-go'
          AND json_extract(data, '$.role') = 'assistant'
          AND json_type(data, '$.cost') IN ('integer', 'real')
        "#
    };
    let mut statement = conn.prepare(sql)?;
    let rows = statement.query_map([], |row| {
        Ok(LocalUsageRow {
            created_ms: row.get(0)?,
            cost: row.get(1)?,
        })
    })?;
    Ok(rows
        .filter_map(Result::ok)
        .filter(|row| row.created_ms > 0 && row.cost >= 0.0 && row.cost.is_finite())
        .collect())
}

fn sqlite_table_exists(conn: &rusqlite::Connection, table: &str) -> Result<bool, rusqlite::Error> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
}

fn parse_usage_data(
    text: &str,
    id: ProviderId,
    source: FetchSource,
    source_label: Option<String>,
) -> ProviderResult {
    let (rolling, weekly, monthly) = parse_usage_json(text).unwrap_or_else(|| {
        (
            extract_usage_window(text, "rollingUsage"),
            extract_usage_window(text, "weeklyUsage"),
            extract_usage_window(text, "monthlyUsage"),
        )
    });

    if rolling.is_none() && weekly.is_none() && monthly.is_none() {
        return ProviderResult {
            provider: id,
            name: OPENCODE_NAME.to_string(),
            status: ProviderStatus::Unavailable,
            error: Some(
                "Could not parse usage data from opencode.ai response; page may not contain Go subscription data"
                    .to_string(),
            ),
            plan: Some(OPENCODE_GO_PLAN.to_string()),
            source: Some(source),
            source_label,
            windows: vec![],
            credits: vec![],
            metrics: vec![],
            fetched_at: chrono::Local::now(),
        };
    }

    let mut windows = Vec::new();
    let now_ts = chrono::Utc::now().timestamp();

    if let Some((pct, reset_secs)) = rolling {
        windows.push(QuotaWindow {
            wtype: WindowType::FiveHours,
            label: "5h".to_string(),
            used_percent: Some(pct.clamp(0.0, 100.0)),
            limit: 100,
            remaining: (100.0 - pct).clamp(0.0, 100.0).round() as i64,
            resets_at_epoch: Some(now_ts + reset_secs as i64),
            resets_at_str: None,
            has_real_remaining: false,
            is_synthetic: false,
        });
    }

    if let Some((pct, reset_secs)) = weekly {
        windows.push(QuotaWindow {
            wtype: WindowType::Weekly,
            label: "Weekly".to_string(),
            used_percent: Some(pct.clamp(0.0, 100.0)),
            limit: 100,
            remaining: (100.0 - pct).clamp(0.0, 100.0).round() as i64,
            resets_at_epoch: Some(now_ts + reset_secs as i64),
            resets_at_str: None,
            has_real_remaining: false,
            is_synthetic: false,
        });
    }

    if let Some((pct, reset_secs)) = monthly {
        windows.push(QuotaWindow {
            wtype: WindowType::Monthly,
            label: "Monthly".to_string(),
            used_percent: Some(pct.clamp(0.0, 100.0)),
            limit: 100,
            remaining: (100.0 - pct).clamp(0.0, 100.0).round() as i64,
            resets_at_epoch: Some(now_ts + reset_secs as i64),
            resets_at_str: None,
            has_real_remaining: false,
            is_synthetic: false,
        });
    }

    ProviderResult {
        provider: id,
        name: OPENCODE_NAME.to_string(),
        status: ProviderStatus::Available,
        error: None,
        plan: Some(OPENCODE_GO_PLAN.to_string()),
        source: Some(source),
        source_label,
        windows,
        credits: vec![],
        metrics: vec![],
        fetched_at: chrono::Local::now(),
    }
}

type UsageWindow = (f64, u64);
type UsageWindows = (
    Option<UsageWindow>,
    Option<UsageWindow>,
    Option<UsageWindow>,
);

fn parse_usage_json(text: &str) -> Option<UsageWindows> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut candidates = Vec::new();
    collect_window_candidates(&value, &mut Vec::new(), &mut candidates);
    if candidates.is_empty() {
        return parse_usage_json_value(&value);
    }

    let weekly = pick_window_candidate(
        &candidates,
        |path| path.contains("weekly") || path.contains("week"),
        false,
    );
    let monthly = pick_window_candidate(
        &candidates,
        |path| path.contains("monthly") || path.contains("month"),
        false,
    );
    let rolling = pick_window_candidate(
        &candidates,
        |path| {
            path.contains("rolling")
                || path.contains("5h")
                || path.contains("5-hour")
                || path.contains("hour")
        },
        true,
    )
    .or_else(|| {
        pick_window_candidate(
            &candidates,
            |path| !path.contains("week") && !path.contains("month") && !path.contains("renew"),
            true,
        )
    });
    (rolling.is_some() || weekly.is_some() || monthly.is_some())
        .then_some((rolling, weekly, monthly))
}

fn parse_usage_json_value(value: &serde_json::Value) -> Option<UsageWindows> {
    if let Some(object) = value.as_object() {
        let rolling = first_window(
            object,
            &[
                "rollingUsage",
                "rolling",
                "rolling_usage",
                "rollingWindow",
                "rolling_window",
            ],
        );
        let weekly = first_window(
            object,
            &[
                "weeklyUsage",
                "weekly",
                "weekly_usage",
                "weeklyWindow",
                "weekly_window",
            ],
        );
        let monthly = first_window(
            object,
            &[
                "monthlyUsage",
                "monthly",
                "monthly_usage",
                "monthlyWindow",
                "monthly_window",
            ],
        );
        if rolling.is_some() || weekly.is_some() || monthly.is_some() {
            return Some((rolling, weekly, monthly));
        }

        for key in ["data", "result", "usage", "billing", "payload"] {
            if let Some(found) = object.get(key).and_then(parse_usage_json_value) {
                return Some(found);
            }
        }

        for child in object.values() {
            if let Some(found) = parse_usage_json_value(child) {
                return Some(found);
            }
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            if let Some(found) = parse_usage_json_value(child) {
                return Some(found);
            }
        }
    }
    None
}

fn first_window(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<(f64, u64)> {
    for key in keys {
        if let Some(window) = object.get(*key).and_then(parse_window_value) {
            return Some(window);
        }
    }
    None
}

fn parse_window_value(value: &serde_json::Value) -> Option<(f64, u64)> {
    if let Some(object) = value.as_object() {
        if let Some(nested) = object.get("window").and_then(parse_window_value) {
            return Some(nested);
        }
        let mut percent = first_f64(
            object,
            &[
                "usagePercent",
                "usage_percent",
                "percent",
                "usedPercent",
                "used_percent",
            ],
        );
        if percent.is_none() {
            let used = first_f64(
                object,
                &["used", "usage", "consumed", "count", "usedTokens"],
            );
            let limit = first_f64(
                object,
                &["limit", "total", "quota", "max", "cap", "tokenLimit"],
            );
            if let (Some(used), Some(limit)) = (used, limit) {
                if limit > 0.0 {
                    percent = Some((used / limit) * 100.0);
                }
            }
        }
        let reset = first_i64_obj(
            object,
            &[
                "resetInSec",
                "reset_in_sec",
                "resetSeconds",
                "reset_seconds",
            ],
        )
        .or_else(|| {
            first_i64_obj(object, &["resetAt", "reset_at", "resetsAt", "resets_at"]).map(|epoch| {
                let epoch = if epoch > 10_000_000_000 {
                    epoch / 1000
                } else {
                    epoch
                };
                (epoch - chrono::Utc::now().timestamp()).max(0)
            })
        })?;
        let percent = percent?;
        let percent = if (0.0..=1.0).contains(&percent) {
            percent * 100.0
        } else {
            percent
        };
        return Some((percent.clamp(0.0, 100.0), reset.max(0) as u64));
    }
    None
}

fn collect_window_candidates(
    value: &serde_json::Value,
    path: &mut Vec<String>,
    output: &mut Vec<(String, (f64, u64))>,
) {
    if let Some(window) = parse_window_value(value) {
        output.push((path.join(".").to_ascii_lowercase(), window));
    }
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                path.push(key.clone());
                collect_window_candidates(child, path, output);
                path.pop();
            }
        }
        serde_json::Value::Array(array) => {
            for (index, child) in array.iter().enumerate() {
                path.push(format!("[{}]", index));
                collect_window_candidates(child, path, output);
                path.pop();
            }
        }
        _ => {}
    }
}

fn pick_window_candidate(
    candidates: &[(String, (f64, u64))],
    matches: impl Fn(&str) -> bool,
    shortest: bool,
) -> Option<(f64, u64)> {
    candidates
        .iter()
        .filter(|(path, _)| matches(path))
        .map(|(_, window)| *window)
        .min_by(|left, right| {
            let ordering = left.1.cmp(&right.1);
            if shortest {
                ordering
            } else {
                ordering.reverse()
            }
        })
}

fn first_f64(object: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_i64().map(|v| v as f64))
                .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
                .filter(|value| value.is_finite())
        })
    })
}

fn first_i64_obj(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<i64> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
                .or_else(|| value.as_f64().map(|v| v.round() as i64))
                .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
        })
    })
}

fn extract_usage_window(text: &str, field_name: &str) -> Option<(f64, u64)> {
    let escaped = regex_lite::escape(field_name);

    let patterns = [
        format!(
            r#"{escaped}:\$R\[\d+\]=\{{[^}}]*usagePercent:(\d+(?:\.\d+)?)[^}}]*resetInSec:(\d+)[^}}]*\}}"#
        ),
        format!(
            r#"{escaped}:\$R\[\d+\]=\{{[^}}]*resetInSec:(\d+)[^}}]*usagePercent:(\d+(?:\.\d+)?)[^}}]*\}}"#
        ),
        format!(
            r#"[\"']?{escaped}[\"']?\s*[:=][^{{}}]*\{{[^}}]*[\"']?usagePercent[\"']?\s*[:=]\s*(\d+(?:\.\d+)?)[^}}]*[\"']?resetInSec[\"']?\s*[:=]\s*(\d+)[^}}]*\}}"#
        ),
        format!(
            r#"[\"']?{escaped}[\"']?\s*[:=][^{{}}]*\{{[^}}]*[\"']?resetInSec[\"']?\s*[:=]\s*(\d+)[^}}]*[\"']?usagePercent[\"']?\s*[:=]\s*(\d+(?:\.\d+)?)[^}}]*\}}"#
        ),
    ];

    for (idx, pattern) in patterns.iter().enumerate() {
        let Some(caps) = regex_lite::Regex::new(pattern).ok()?.captures(text) else {
            continue;
        };
        if idx == 1 || idx == 3 {
            let reset: u64 = caps.get(1)?.as_str().parse().ok()?;
            let pct = caps.get(2)?.as_str().parse::<f64>().ok()?;
            return Some((pct, reset));
        }
        let pct = caps.get(1)?.as_str().parse::<f64>().ok()?;
        let reset: u64 = caps.get(2)?.as_str().parse().ok()?;
        return Some((pct, reset));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_opencode_nested_json_usage() {
        let text = r#"{
            "usage": {
                "rollingUsage": {"usagePercent": 0.25, "resetInSec": 3600},
                "weeklyUsage": {"window": {"usagePercent": 75, "resetInSec": 7200}},
                "monthlyUsage": {"usagePercent": "91", "resetInSec": "880201"}
            }
        }"#;

        let parsed = parse_usage_json(text).unwrap();
        assert_eq!(parsed.0, Some((25.0, 3600)));
        assert_eq!(parsed.1, Some((75.0, 7200)));
        assert_eq!(parsed.2, Some((91.0, 880201)));
    }

    #[test]
    fn combines_windows_from_sibling_objects() {
        let text = include_str!("../../tests/fixtures/opencode/sibling-windows.json");

        let parsed = parse_usage_json(text).unwrap();
        assert_eq!(parsed.0, Some((12.5, 600)));
        assert_eq!(parsed.1, Some((44.25, 9000)));
        assert_eq!(parsed.2, Some((73.0, 50000)));
    }

    #[test]
    fn local_history_only_reads_opencode_go_messages() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY,
                data TEXT NOT NULL,
                time_created INTEGER NOT NULL
            );",
        )
        .unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        for (id, provider, cost) in [("go", "opencode-go", 2.5), ("other", "anthropic", 99.0)] {
            let data = serde_json::json!({
                "providerID": provider,
                "role": "assistant",
                "cost": cost,
                "time": {"created": now}
            })
            .to_string();
            conn.execute(
                "INSERT INTO message (id, data, time_created) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, data, now],
            )
            .unwrap();
        }

        let rows = read_local_usage_rows(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cost, 2.5);
    }

    #[test]
    fn local_cost_windows_are_marked_as_estimates() {
        let window = local_cost_window("5h", WindowType::FiveHours, 3.0, 12.0, 1234);

        assert_eq!(window.used_percent, Some(25.0));
        assert!(!window.has_real_remaining);
        assert!(window.is_synthetic);
    }
}
