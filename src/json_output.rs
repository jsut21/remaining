use crate::types::{ProviderResult, ProviderStatus, WindowType};
use serde_json::{json, Value};

fn window_type_str(w: &WindowType) -> &'static str {
    match w {
        WindowType::FiveHours => "5h",
        WindowType::Weekly => "weekly",
        WindowType::Monthly => "monthly",
        WindowType::Other => "other",
    }
}

fn status_str(s: &ProviderStatus) -> &'static str {
    match s {
        ProviderStatus::Available => "available",
        ProviderStatus::AuthRequired => "auth_required",
        ProviderStatus::NetworkError => "network_error",
        ProviderStatus::Unavailable => "unavailable",
    }
}

pub fn json_print_results(results: &[ProviderResult], pretty: bool) {
    let arr: Vec<Value> = results
        .iter()
        .map(|r| {
            let windows: Vec<Value> = r
                .windows
                .iter()
                .map(|w| {
                    let mut obj = json!({
                        "type": window_type_str(&w.wtype),
                        "label": w.label,
                        "used_percent": w.used_percent,
                        "remaining": w.remaining,
                        "limit": w.limit,
                    });
                    if let Some(ref s) = w.resets_at_str {
                        obj["resets_at"] = json!(s);
                    }
                    if let Some(e) = w.resets_at_epoch {
                        obj["resets_at_epoch"] = json!(e);
                    }
                    obj
                })
                .collect();

            json!({
                "provider": r.provider.as_str(),
                "name": r.name,
                "status": status_str(&r.status),
                "plan": r.plan,
                "error": r.error,
                "windows": windows,
                "fetched_at": r.fetched_at.to_rfc3339(),
            })
        })
        .collect();

    let output = json!(arr);
    if pretty {
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        println!("{}", serde_json::to_string(&output).unwrap());
    }
}