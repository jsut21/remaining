use crate::engine::FetchOptions;
use crate::engine::snapshot::{
    FetchSource, ProviderId, ProviderResult, ProviderStatus, QuotaWindow, WindowType,
};
use crate::utils::{home_dir, network_error_message};

pub async fn fetch(options: &FetchOptions) -> ProviderResult {
    let timeout = &options.timeout;
    let id = ProviderId::Ollama;

    let cookie = match find_cookie(options.allow_browser_cookies) {
        Some(c) => c,
        None => {
            return ProviderResult {
                provider: id,
                name: "Ollama".to_string(),
                status: ProviderStatus::AuthRequired,
                error: Some("No session cookie found. Sign in to ollama.com in Firefox without --no-browser-cookies, set OLLAMA_COOKIE_HEADER or OLLAMA_SESSION_COOKIE, or install ollama-usage".into()),
                plan: None,
                source: Some(FetchSource::BrowserCookie),
                source_label: None,
                windows: vec![],
                credits: vec![],
                metrics: vec![],
                fetched_at: chrono::Local::now(),
            };
        }
    };

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let resp = client
        .get("https://ollama.com/settings")
        .header("Cookie", cookie.header.clone())
        .header("User-Agent", "Mozilla/5.0")
        .timeout(*timeout.as_ref())
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return ProviderResult::network_error(id, network_error_message(&e)),
    };

    let status = resp.status();
    let final_url = resp.url().clone();
    if status.as_u16() == 401 || status.as_u16() == 403 || is_sign_in_url(final_url.as_str()) {
        return auth_required_result(id, cookie.source_label);
    }
    if !status.is_success() {
        return ProviderResult::unavailable(
            id,
            format!("Ollama settings returned HTTP {}", status.as_u16()),
        );
    }

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => return ProviderResult::unavailable(id, network_error_message(&e)),
    };

    parse_ollama_settings(&body, id, cookie.source_label)
}

struct CookieHeader {
    header: String,
    source_label: String,
}

fn find_cookie(allow_browser_cookies: bool) -> Option<CookieHeader> {
    if let Some(header) = read_env_trim(&["OLLAMA_COOKIE_HEADER", "OLLAMA_COOKIE"]) {
        let header = normalize_cookie_header(&header, "__Secure-session");
        if has_recognized_session_cookie(&header) {
            return Some(CookieHeader {
                header,
                source_label: "manual cookie header".to_string(),
            });
        }
    }

    if let Some(value) = read_env_trim(&["OLLAMA_SESSION_COOKIE"]) {
        return Some(CookieHeader {
            header: normalize_cookie_header(&value, "__Secure-session"),
            source_label: "OLLAMA_SESSION_COOKIE".to_string(),
        });
    }

    if let Some(home) = home_dir() {
        let cookie_file = home.join(".config/ollama-usage/session");
        if let Ok(content) = std::fs::read_to_string(&cookie_file) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return Some(CookieHeader {
                    header: normalize_cookie_header(trimmed, "__Secure-session"),
                    source_label: "~/.config/ollama-usage/session".to_string(),
                });
            }
        }
    }

    if allow_browser_cookies {
        find_firefox_cookie()
    } else {
        None
    }
}

fn find_firefox_cookie() -> Option<CookieHeader> {
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
                let Ok(mut statement) = conn.prepare(
                    "SELECT name, value FROM moz_cookies
                     WHERE (host='.ollama.com' OR host='ollama.com')
                       AND (
                         name IN ('session','wos-session','__Secure-session','ollama_session','__Host-ollama_session','ollama-session','next-auth.session-token','__Secure-next-auth.session-token')
                         OR name LIKE 'next-auth.session-token.%'
                         OR name LIKE '__Secure-next-auth.session-token.%'
                       )
                     ORDER BY name ASC",
                ) else {
                    continue;
                };
                let Ok(rows) = statement.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                }) else {
                    continue;
                };
                let cookies = rows.filter_map(Result::ok).collect::<Vec<_>>();
                if !cookies.is_empty() {
                    let names = cookies
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Some(CookieHeader {
                        header: cookies
                            .into_iter()
                            .map(|(name, value)| format!("{}={}", name, value))
                            .collect::<Vec<_>>()
                            .join("; "),
                        source_label: format!("Firefox cookies ({})", names),
                    });
                }
            }
        }
    }

    None
}

fn normalize_cookie_header(raw: &str, default_name: &str) -> String {
    let trimmed = raw.trim().trim_start_matches("Cookie:").trim();
    if trimmed.contains('=') {
        trimmed.to_string()
    } else {
        format!("{}={}", default_name, trimmed)
    }
}

fn has_recognized_session_cookie(header: &str) -> bool {
    header.split(';').any(|pair| {
        let name = pair.split('=').next().unwrap_or("").trim();
        matches!(
            name,
            "session"
                | "wos-session"
                | "__Secure-session"
                | "ollama_session"
                | "__Host-ollama_session"
                | "ollama-session"
                | "next-auth.session-token"
                | "__Secure-next-auth.session-token"
        ) || name.starts_with("next-auth.session-token.")
            || name.starts_with("__Secure-next-auth.session-token.")
    })
}

fn is_sign_in_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("ollama.com/signin")
        || lower.contains("signin.ollama.com")
        || lower.contains("workos.com") && lower.contains("authorize")
}

fn auth_required_result(id: ProviderId, source_label: String) -> ProviderResult {
    ProviderResult {
        provider: id,
        name: "Ollama".to_string(),
        status: ProviderStatus::AuthRequired,
        error: Some("Ollama session cookie appears to be expired".into()),
        plan: None,
        source: Some(FetchSource::BrowserCookie),
        source_label: Some(source_label),
        windows: vec![],
        credits: vec![],
        metrics: vec![],
        fetched_at: chrono::Local::now(),
    }
}

fn read_env_trim(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn parse_ollama_settings(html: &str, id: ProviderId, source_label: String) -> ProviderResult {
    if html.contains("/signin") || html.contains("workos") && html.contains("authorize") {
        return auth_required_result(id, source_label);
    }

    let plan_pattern = r#"(?is)Cloud\s+Usage\s*</span>\s*<span[^>]*>([^<]+)</span>"#;
    let mut plan = regex_lite::Regex::new(plan_pattern).ok().and_then(|regex| {
        regex.captures(html).and_then(|captures| {
            captures.get(1).and_then(|value| {
                let plan = value.as_str().trim();
                (!plan.is_empty()).then(|| capitalize_first(plan))
            })
        })
    });
    let mut session_pct: Option<f64> = None;
    let mut weekly_pct: Option<f64> = None;
    let mut session_epoch: Option<i64> = None;
    let mut weekly_epoch: Option<i64> = None;

    if plan.is_none() {
        if let Some(idx) = html.find("\"plan\":") {
            let slice = safe_byte_limited_slice(html, idx, 100);
            plan = regex_lite::Regex::new(r#""plan":\s*"([^"]+)""#)
                .ok()
                .and_then(|regex| regex.captures(slice))
                .and_then(|captures| captures.get(1).map(|value| value.as_str().to_string()));
        }
    }

    let session_idx = html.find("Session usage");
    let hourly_idx = html.find("Hourly usage");
    let primary_idx = session_idx.or(hourly_idx);
    let weekly_idx = html.find("Weekly usage");

    if let Some(s_idx) = primary_idx {
        let nearby = if let Some(w_idx) = weekly_idx.filter(|w| *w > s_idx) {
            html.get(s_idx..w_idx).unwrap_or("")
        } else {
            safe_byte_limited_slice(html, s_idx, 10000)
        };
        session_pct = extract_percentage(nearby);
        session_epoch = extract_reset_epoch(nearby);
    }

    if let Some(w_idx) = weekly_idx {
        let nearby = safe_byte_limited_slice(html, w_idx, 10000);
        weekly_pct = extract_percentage(nearby);
        weekly_epoch = extract_reset_epoch(nearby);
    }

    let mut windows = Vec::new();

    if let Some(pct) = session_pct {
        let (wtype, label) = if session_idx.is_some() {
            (WindowType::FiveHours, "5h")
        } else {
            (WindowType::Other, "Hourly")
        };
        windows.push(QuotaWindow {
            wtype,
            label: label.to_string(),
            used_percent: Some(pct.clamp(0.0, 100.0)),
            limit: 100,
            remaining: (100.0 - pct).clamp(0.0, 100.0).round() as i64,
            resets_at_epoch: session_epoch,
            resets_at_str: None,
            has_real_remaining: false,
            is_synthetic: false,
        });
    }

    if let Some(pct) = weekly_pct {
        windows.push(QuotaWindow {
            wtype: WindowType::Weekly,
            label: "Weekly".to_string(),
            used_percent: Some(pct.clamp(0.0, 100.0)),
            limit: 100,
            remaining: (100.0 - pct).clamp(0.0, 100.0).round() as i64,
            resets_at_epoch: weekly_epoch,
            resets_at_str: None,
            has_real_remaining: false,
            is_synthetic: false,
        });
    }

    if windows.is_empty() {
        return ProviderResult {
            provider: id,
            name: "Ollama".to_string(),
            status: ProviderStatus::Unavailable,
            error: Some("Could not parse usage from ollama.com/settings".into()),
            plan,
            source: Some(FetchSource::BrowserCookie),
            source_label: Some(source_label),
            windows: vec![],
            credits: vec![],
            metrics: vec![],
            fetched_at: chrono::Local::now(),
        };
    }

    ProviderResult {
        provider: id,
        name: "Ollama".to_string(),
        status: ProviderStatus::Available,
        error: None,
        plan,
        source: Some(FetchSource::BrowserCookie),
        source_label: Some(source_label),
        windows,
        credits: vec![],
        metrics: vec![],
        fetched_at: chrono::Local::now(),
    }
}

fn capitalize_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn safe_byte_limited_slice(text: &str, start: usize, max_bytes: usize) -> &str {
    let Some(tail) = text.get(start..) else {
        return "";
    };

    let mut end = tail.len().min(max_bytes);
    while end > 0 && !tail.is_char_boundary(end) {
        end -= 1;
    }
    &tail[..end]
}

fn extract_percentage(text: &str) -> Option<f64> {
    let patterns = [
        r"(?i)(\d+(?:\.\d+)?)%\s*used",
        r"(?i)used\s*(\d+(?:\.\d+)?)%",
    ];
    for pattern in patterns {
        if let Some(value) = percentage_capture(pattern, text) {
            return Some(value);
        }
    }
    percentage_capture(r"width:\s*(\d+(?:\.\d+)?)%", text)
}

fn percentage_capture(pattern: &str, text: &str) -> Option<f64> {
    let value = regex_lite::Regex::new(pattern)
        .ok()?
        .captures(text)?
        .get(1)?
        .as_str()
        .parse::<f64>()
        .ok()?;
    value.is_finite().then_some(value.clamp(0.0, 100.0))
}

fn extract_reset_epoch(text: &str) -> Option<i64> {
    let data_time_re = regex_lite::Regex::new(r#"data-time="([^"]+)""#).ok()?;
    if let Some(timestamp) = data_time_re
        .captures(text)
        .and_then(|captures| captures.get(1))
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value.as_str()).ok())
    {
        return Some(timestamp.timestamp());
    }

    let text_re = regex_lite::Regex::new(
        r"(?i)Resets?\s+in\s+(\d+)\s*(seconds?|secs?|s|minutes?|mins?|m|hours?|hrs?|h|days?|d)",
    )
    .ok()?;
    if let Some(caps) = text_re.captures(text) {
        let n: i64 = caps.get(1)?.as_str().parse().ok()?;
        let unit = caps.get(2)?.as_str().to_lowercase();
        let mult: i64 = match unit.as_str() {
            value if value.starts_with('s') => 1,
            value if value.starts_with('m') => 60,
            value if value.starts_with('h') => 3600,
            value if value.starts_with('d') => 86400,
            _ => return None,
        };
        return Some(chrono::Utc::now().timestamp() + n * mult);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_byte_limited_slice_does_not_split_utf8() {
        let text = "Weekly usage".to_string() + &"a".repeat(9_987) + "© 20% used";
        let idx = text.find("Weekly usage").unwrap();

        let slice = safe_byte_limited_slice(&text, idx, 10_000);

        assert!(slice.ends_with('a'));
    }

    #[test]
    fn parses_weekly_usage_when_html_has_multibyte_after_window() {
        let html = "<h2>Weekly usage</h2><div>20% used</div>".to_string()
            + &"a".repeat(9_955)
            + "© footer";

        let result = parse_ollama_settings(&html, ProviderId::Ollama, "test".to_string());

        assert_eq!(result.windows.len(), 1);
        assert_eq!(result.windows[0].used_percent, Some(20.0));
    }

    #[test]
    fn parses_cloud_usage_fixture() {
        let html = r#"
        <div>
          <h2 class="text-xl">
            <span>Cloud Usage</span>
            <span class="text-xs">free</span>
          </h2>
          <div>
            <span>Session usage</span>
            <span>0.1% used</span>
            <div class="local-time" data-time="2026-01-30T18:00:00Z">Resets in 3 hours</div>
          </div>
          <div>
            <span>Weekly usage</span>
            <span>0.7% Used</span>
            <div class="local-time" data-time="2026-02-02T00:00:00Z">Resets in 2 days</div>
          </div>
        </div>
        "#;

        let result = parse_ollama_settings(html, ProviderId::Ollama, "test".to_string());

        assert_eq!(result.plan.as_deref(), Some("Free"));
        assert_eq!(result.windows.len(), 2);
        assert_eq!(result.windows[0].used_percent, Some(0.1));
        assert_eq!(result.windows[1].used_percent, Some(0.7));
        assert_eq!(result.windows[0].resets_at_epoch, Some(1769796000));
        assert_eq!(result.windows[1].resets_at_epoch, Some(1769990400));
    }

    #[test]
    fn ollama_plan_ignores_later_billing_text() {
        let html = r#"
        <div>
          <h2 class="text-xl">
            <span>Cloud Usage</span>
            <span class="text-xs">free</span>
          </h2>
          <div><span>Session usage</span><span>0.1% used</span></div>
          <div class="capitalize">Automatically add $5 to your balance when it hits $0</div>
        </div>
        "#;

        let result = parse_ollama_settings(html, ProviderId::Ollama, "test".to_string());

        assert_eq!(result.plan.as_deref(), Some("Free"));
    }

    #[test]
    fn parses_hourly_usage_as_primary_window() {
        let html = include_str!("../../tests/fixtures/ollama/hourly-usage.html");

        let result = parse_ollama_settings(html, ProviderId::Ollama, "test".to_string());

        assert_eq!(result.windows.len(), 2);
        assert_eq!(result.windows[0].used_percent, Some(2.5));
        assert_eq!(result.windows[0].wtype, WindowType::Other);
        assert_eq!(result.windows[0].label, "Hourly");
        assert_eq!(result.windows[1].used_percent, Some(4.2));
    }

    #[test]
    fn clamps_out_of_range_usage() {
        let html = "<span>Session usage</span><span>140% used</span>";
        let result = parse_ollama_settings(html, ProviderId::Ollama, "test".to_string());

        assert_eq!(result.windows[0].used_percent, Some(100.0));
    }
}
