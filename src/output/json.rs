use crate::engine::snapshot::{
    CreditBalance, FetchSource, ProviderId, ProviderResult, ProviderStatus, QuotaUnit, QuotaWindow,
    UsageMetric, WindowType,
};
use serde_json::{Value, json};

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

fn inferred_unit(provider: ProviderId, window: &QuotaWindow) -> QuotaUnit {
    if !window.has_real_remaining {
        return QuotaUnit::Percent;
    }

    match provider {
        ProviderId::Zai if matches!(window.wtype, WindowType::FiveHours | WindowType::Weekly) => {
            QuotaUnit::Tokens
        }
        ProviderId::Zai if window.label.to_ascii_lowercase().contains("token") => QuotaUnit::Tokens,
        ProviderId::Zai => QuotaUnit::Units,
        ProviderId::MiniMax => QuotaUnit::Units,
        _ => QuotaUnit::Units,
    }
}

fn window_json(provider: ProviderId, window: &QuotaWindow) -> Value {
    let unit = inferred_unit(provider, window);
    let remaining_percent = if window.has_real_remaining && window.limit > 0 {
        Some((window.remaining as f64 * 100.0 / window.limit as f64).clamp(0.0, 100.0))
    } else if window.has_real_remaining {
        window
            .used_percent
            .map(|used| (100.0 - used).clamp(0.0, 100.0))
    } else {
        window
            .used_percent
            .map(|used| (100.0 - used).clamp(0.0, 100.0))
    };
    let used_percent = window.used_percent.map(|used| used.clamp(0.0, 100.0));

    let mut obj = json!({
        "type": window_type_str(&window.wtype),
        "label": window.label,
        "used_percent": used_percent,
        "usage_known": used_percent.is_some(),
        "is_synthetic": window.is_synthetic,
        "remaining": window.remaining,
        "limit": window.limit,
        "remaining_percent": remaining_percent,
        "used_units": if window.has_real_remaining { Some(window.limit - window.remaining) } else { None },
        "remaining_units": if window.has_real_remaining { Some(window.remaining) } else { None },
        "limit_units": if window.has_real_remaining { Some(window.limit) } else { None },
        "unit": unit.as_str(),
    });
    if let Some(ref s) = window.resets_at_str {
        obj["resets_at"] = json!(s);
    }
    if let Some(e) = window.resets_at_epoch {
        obj["resets_at_epoch"] = json!(e);
    }
    obj
}

fn metric_json(metric: &UsageMetric) -> Value {
    json!({
        "label": metric.label,
        "value": metric.value,
        "unit": metric.unit.as_str(),
        "period": metric.period,
        "source": metric.source_label,
    })
}

fn credit_json(credit: &CreditBalance) -> Value {
    let mut obj = json!({
        "label": credit.label,
        "remaining": credit.remaining,
        "total": credit.total,
        "unit": credit.unit.as_str(),
        "source": credit.source_label,
    });
    if let Some(expires) = credit.expires_at_epoch {
        obj["expires_at_epoch"] = json!(expires);
    }
    obj
}

fn results_json(results: &[ProviderResult]) -> Value {
    let arr: Vec<Value> = results
        .iter()
        .map(|r| {
            let windows: Vec<Value> = r
                .windows
                .iter()
                .map(|w| window_json(r.provider, w))
                .collect();
            let credits: Vec<Value> = r.credits.iter().map(credit_json).collect();
            let metrics: Vec<Value> = r.metrics.iter().map(metric_json).collect();
            let source = r.source.unwrap_or(FetchSource::Unknown);

            json!({
                "schema_version": 2,
                "provider": r.provider.as_str(),
                "name": r.name,
                "status": status_str(&r.status),
                "plan": r.plan,
                "source": source.as_str(),
                "source_label": r.source_label,
                "error": r.error,
                "windows": windows,
                "credits": credits,
                "metrics": metrics,
                "fetched_at": r.fetched_at.to_rfc3339(),
            })
        })
        .collect();
    json!(arr)
}

pub fn json_print_results(results: &[ProviderResult], pretty: bool) {
    let output = results_json(results);
    if pretty {
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        println!("{}", serde_json::to_string(&output).unwrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_usage_is_null_instead_of_fake_full_remaining() {
        let result = ProviderResult {
            provider: ProviderId::Zai,
            name: "Z.ai".to_string(),
            status: ProviderStatus::Available,
            error: None,
            plan: Some("test".to_string()),
            source: Some(FetchSource::ApiKey),
            source_label: Some("fixture".to_string()),
            windows: vec![QuotaWindow {
                wtype: WindowType::FiveHours,
                label: "5h".to_string(),
                used_percent: None,
                limit: 0,
                remaining: 0,
                resets_at_epoch: None,
                resets_at_str: None,
                has_real_remaining: false,
                is_synthetic: false,
            }],
            credits: vec![],
            metrics: vec![],
            fetched_at: chrono::Local::now(),
        };

        let output = results_json(&[result]);
        assert_eq!(output[0]["schema_version"], 2);
        assert_eq!(output[0]["windows"][0]["used_percent"], Value::Null);
        assert_eq!(output[0]["windows"][0]["remaining_percent"], Value::Null);
        assert_eq!(output[0]["windows"][0]["usage_known"], false);
    }
}
