use crate::engine::snapshot::{ProviderResult, ProviderStatus, QuotaWindow};
use crate::utils::progress_bar;
use console::Style;

fn format_reset(window: &QuotaWindow) -> String {
    if let Some(ref s) = window.resets_at_str {
        return format!("resets {}", s);
    }
    if let Some(dt) = window
        .resets_at_epoch
        .and_then(|epoch| chrono::DateTime::from_timestamp(epoch, 0))
    {
        let now = chrono::Utc::now();
        let diff = dt.signed_duration_since(now);
        if diff.num_seconds() > 0 {
            let total_hours = diff.num_hours();
            let days = total_hours / 24;
            let hours = total_hours % 24;
            let mins = diff.num_minutes() % 60;
            if days > 0 {
                return format!("resets in {}d {}h", days, hours);
            }
            if hours > 0 {
                return format!("resets in {}h {}m", hours, mins);
            }
            return format!("resets in {}m", diff.num_minutes());
        }
        return format!(
            "resets {}",
            dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
        );
    }
    String::new()
}

pub fn display_result(result: &ProviderResult, use_color: bool) {
    let green = Style::new().green();
    let yellow = Style::new().yellow();
    let red = Style::new().red();
    let bold = Style::new().bold();
    let dim = Style::new().dim();

    let any_exhausted = result
        .windows
        .iter()
        .any(|window| window.used_percent.is_some_and(|used| used >= 100.0));

    let status_icon = match result.status {
        ProviderStatus::Available if any_exhausted => "🚫",
        ProviderStatus::Available => "✅",
        ProviderStatus::AuthRequired => "🔐",
        ProviderStatus::NetworkError => "🌐",
        ProviderStatus::Unavailable => "⚠️",
    };

    let plan_str = result
        .plan
        .as_ref()
        .map(|p| format!(" ({})", p))
        .unwrap_or_default();

    println!();
    if use_color {
        println!(
            "  {} {}{}",
            status_icon,
            bold.apply_to(&result.name),
            dim.apply_to(&plan_str),
        );
    } else {
        println!("  {} {}{}", status_icon, result.name, plan_str);
    }

    if result.status != ProviderStatus::Available {
        if let Some(err) = &result.error {
            if use_color {
                println!("     {}", red.apply_to(err));
            } else {
                println!("     {}", err);
            }
        }
        return;
    }

    for window in &result.windows {
        let Some(pct) = window.used_percent else {
            let reset_str = format_reset(window);
            let reset_display = if reset_str.is_empty() {
                String::new()
            } else {
                format!("  {}", reset_str)
            };
            println!(
                "     {:15} [usage unavailable]{}",
                window.label, reset_display
            );
            continue;
        };
        let pct_label = format_percent(pct);
        let (bar_color, pct_str) = if pct >= 90.0 {
            (&red, red.apply_to(format!("{}%", pct_label)))
        } else if pct >= 70.0 {
            (&yellow, yellow.apply_to(format!("{}%", pct_label)))
        } else {
            (&green, green.apply_to(format!("{}%", pct_label)))
        };

        let reset_str = format_reset(window);

        if use_color {
            let reset_display = if reset_str.is_empty() {
                String::new()
            } else {
                format!("  {}", dim.apply_to(&reset_str))
            };
            println!(
                "     {:15} [{}] {:<5}{}",
                window.label,
                bar_color.apply_to(progress_bar(pct, 20)),
                pct_str,
                reset_display,
            );
        } else {
            let pct_display = format!("{}%", pct_label);
            let reset_display = if reset_str.is_empty() {
                String::new()
            } else {
                format!("  {}", reset_str)
            };
            println!(
                "     {:15} [{}] {:<5}{}",
                window.label,
                progress_bar(pct, 20),
                pct_display,
                reset_display,
            );
        }
    }
}

fn format_percent(percent: f64) -> String {
    if (percent - percent.round()).abs() < 0.05 {
        format!("{:.0}", percent)
    } else {
        format!("{:.1}", percent)
    }
}
