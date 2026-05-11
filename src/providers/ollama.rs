use crate::types::{ProviderId, ProviderResult, ProviderStatus, QuotaWindow, WindowType};
use crate::utils::home_dir;
use std::sync::Arc;
use std::time::Duration;

pub async fn fetch(timeout: &Arc<Duration>) -> ProviderResult {
    let id = ProviderId::Ollama;

    let cookie = match find_cookie() {
        Some(c) => c,
        None => {
            return ProviderResult {
                provider: id,
                name: "Ollama".to_string(),
                status: ProviderStatus::AuthRequired,
                error: Some("No session cookie found. Set OLLAMA_SESSION_COOKIE env var, install ollama-usage, or log in to ollama.com in Firefox".into()),
                plan: None,
                windows: vec![],
                fetched_at: chrono::Local::now(),
            };
        }
    };

    let client = reqwest::Client::new();
    let resp = client
        .get("https://ollama.com/settings")
        .header("Cookie", format!("__Secure-session={}", cookie))
        .header("User-Agent", "Mozilla/5.0")
        .timeout(*timeout.as_ref())
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return ProviderResult::network_error(id, e.to_string()),
    };

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => return ProviderResult::unavailable(id, e.to_string()),
    };

    parse_ollama_settings(&body, id)
}

fn find_cookie() -> Option<String> {
    if let Ok(val) = std::env::var("OLLAMA_SESSION_COOKIE") {
        let trimmed = val.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }

    if let Some(home) = home_dir() {
        let cookie_file = home.join(".config/ollama-usage/session");
        if let Ok(content) = std::fs::read_to_string(&cookie_file) {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }

    if let Some(home) = home_dir() {
        let search_dirs = [
            home.join(".mozilla/firefox"),
            home.join("snap/firefox/common/.mozilla/firefox"),
        ];

        for profiles_dir in &search_dirs {
            if let Ok(entries) = std::fs::read_dir(profiles_dir) {
                for entry in entries.flatten() {
                    let cookies_db = entry.path().join("cookies.sqlite");
                    if cookies_db.exists() {
                        if let Ok(conn) = rusqlite::Connection::open_with_flags(
                            &cookies_db,
                            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                        ) {
                            let result: Result<String, _> = conn.query_row(
                                "SELECT value FROM moz_cookies WHERE (host='.ollama.com' OR host='ollama.com') AND name LIKE '%session%' ORDER BY lastAccessed DESC LIMIT 1",
                                [],
                                |row| row.get(0),
                            );
                            if let Ok(cookie) = result {
                                return Some(cookie);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

fn parse_ollama_settings(html: &str, id: ProviderId) -> ProviderResult {
    let mut plan = None;
    let mut session_pct: Option<i32> = None;
    let mut weekly_pct: Option<i32> = None;
    let mut session_resets: Option<String> = None;
    let mut weekly_resets: Option<String> = None;

    let capitalize_re = regex_lite::Regex::new(r#"capitalize"\s*>([^<]+)</"#).ok();
    if let Some(re) = &capitalize_re {
        if let Some(caps) = re.captures(html) {
            if let Some(m) = caps.get(1) {
                let p = m.as_str().trim();
                if !p.is_empty() {
                    plan = Some(p.chars().next().unwrap().to_uppercase().collect::<String>() + &p[1..]);
                }
            }
        }
    }
    if plan.is_none() {
        if let Some(idx) = html.find("\"plan\":") {
            let slice = &html[idx..html.len().min(idx + 100)];
            let val_re = regex_lite::Regex::new(r#""plan":\s*"([^"]+)""#).ok();
            if let Some(re) = val_re {
                if let Some(caps) = re.captures(slice) {
                    if let Some(m) = caps.get(1) {
                        plan = Some(m.as_str().to_string());
                    }
                }
            }
        }
    }

    if let Some(idx) = html.find("Session usage") {
        let nearby = &html[idx..html.len().min(idx + 600)];
        session_pct = extract_percentage(nearby);
        session_resets = extract_reset(nearby);
    }

    if let Some(idx) = html.find("Weekly usage") {
        let nearby = &html[idx..html.len().min(idx + 600)];
        weekly_pct = extract_percentage(nearby);
        weekly_resets = extract_reset(nearby);
    }

    let mut windows = Vec::new();

    if let Some(pct) = session_pct {
        windows.push(QuotaWindow {
            wtype: WindowType::FiveHours,
            label: "5h".to_string(),
            used_percent: pct,
            limit: 100,
            remaining: (100 - pct) as i64,
            resets_at_epoch: None,
            resets_at_str: session_resets,
            has_real_remaining: false,
        });
    }

    if let Some(pct) = weekly_pct {
        windows.push(QuotaWindow {
            wtype: WindowType::Weekly,
            label: "Weekly".to_string(),
            used_percent: pct,
            limit: 100,
            remaining: (100 - pct) as i64,
            resets_at_epoch: None,
            resets_at_str: weekly_resets,
            has_real_remaining: false,
        });
    }

    if windows.is_empty() {
        return ProviderResult::unavailable(id, "Could not parse usage from ollama.com/settings");
    }

    ProviderResult {
        provider: id,
        name: "Ollama".to_string(),
        status: ProviderStatus::Available,
        error: None,
        plan,
        windows,
        fetched_at: chrono::Local::now(),
    }
}

fn extract_percentage(text: &str) -> Option<i32> {
    let re = regex_lite::Regex::new(r"(\d+(?:\.\d+)?)%\s*used").ok()?;
    if let Some(caps) = re.captures(text) {
        if let Some(m) = caps.get(1) {
            if let Ok(f) = m.as_str().parse::<f64>() {
                return Some(f.round() as i32);
            }
        }
    }
    let re2 = regex_lite::Regex::new(r"width:\s*(\d+(?:\.\d+)?)%").ok()?;
    if let Some(caps) = re2.captures(text) {
        if let Some(m) = caps.get(1) {
            if let Ok(f) = m.as_str().parse::<f64>() {
                return Some(f.round() as i32);
            }
        }
    }
    None
}

fn extract_reset(text: &str) -> Option<String> {
    let data_time_re = regex_lite::Regex::new(r#"data-time="([^"]+)""#).ok()?;
    if let Some(caps) = data_time_re.captures(text) {
        if let Some(m) = caps.get(1) {
            let iso = m.as_str();
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso) {
                let now = chrono::Utc::now();
                let diff = dt.signed_duration_since(now);
                if diff.num_seconds() > 0 {
                    let total_hours = diff.num_hours();
                    let days = total_hours / 24;
                    let hours = total_hours % 24;
                    let mins = diff.num_minutes() % 60;
                    if days > 0 {
                        return Some(format!("in {}d {}h", days, hours));
                    }
                    if hours > 0 {
                        return Some(format!("in {}h {}m", hours, mins));
                    }
                    return Some(format!("in {}m", diff.num_minutes()));
                }
                return Some(iso.to_string());
            }
        }
    }
    let text_re = regex_lite::Regex::new(r"(?i)Resets?\s+in\s+\d+\s*(?:hour|minute|day|sec)[s>]?").ok()?;
    if let Some(caps) = text_re.captures(text) {
        return Some(caps[0].to_string());
    }
    None
}