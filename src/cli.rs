use crate::engine::FetchOptions;
use crate::engine::registry;
use crate::engine::snapshot::{ProviderResult, WindowType};
use crate::output;
use clap::Parser;
use std::num::NonZeroU64;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "remaining")]
#[command(version)]
#[command(about = "A small CLI and GNOME panel indicator for AI usage quotas.")]
pub struct Cli {
    #[arg(long, help = "Output in JSON format")]
    json: bool,

    #[arg(long, requires = "json", help = "Pretty-print JSON output")]
    pretty: bool,

    #[arg(
        long,
        value_delimiter = ',',
        help = "Only check specific providers. Example: --provider codex,zai,ollama"
    )]
    provider: Vec<String>,

    #[arg(
        long,
        conflicts_with = "provider",
        help = "Check every supported provider"
    )]
    all: bool,

    // Retained for compatibility with existing scripts.
    #[arg(long, conflicts_with = "no_browser_cookies", hide = true)]
    browser_cookies: bool,

    #[arg(
        long,
        conflicts_with = "browser_cookies",
        help = "Disable reading cookies from local browser profiles"
    )]
    no_browser_cookies: bool,

    #[arg(
        long,
        help = "Allow credentials to be sent to custom HTTPS provider endpoints"
    )]
    allow_custom_endpoints: bool,

    #[arg(long, help = "Disable colored output")]
    no_color: bool,

    #[arg(long, help = "Auto-refresh every non-zero N seconds")]
    watch: Option<NonZeroU64>,

    #[arg(
        long,
        default_value = "15",
        help = "Request timeout in non-zero seconds"
    )]
    timeout: NonZeroU64,

    #[arg(long, help = "List supported providers and exit")]
    list_providers: bool,
}

pub async fn run() {
    let cli = Cli::parse();

    if cli.list_providers {
        for provider in registry::PROVIDERS {
            let default_marker = if provider.default_enabled {
                " default"
            } else {
                ""
            };
            let windows = provider
                .windows
                .iter()
                .map(window_type_label)
                .collect::<Vec<_>>()
                .join(",");
            println!(
                "{}\t{}{}\taliases: {}\twindows: {}\tauth: {}",
                provider.id.as_str(),
                provider.display_name,
                default_marker,
                provider.aliases.join(","),
                windows,
                provider.auth_methods.join(", ")
            );
        }
        return;
    }

    let unknown = cli
        .provider
        .iter()
        .filter(|spec| registry::find_by_alias(spec).is_none())
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        eprintln!(
            "Unknown provider(s): {}. Known providers: {}",
            unknown.join(","),
            registry::provider_list_help()
        );
        std::process::exit(2);
    }

    let providers = if cli.all {
        registry::all_provider_ids()
    } else {
        registry::provider_ids_from_specs(&cli.provider)
    };
    if providers.is_empty() {
        eprintln!(
            "No providers selected. Known providers: {}",
            registry::provider_list_help()
        );
        std::process::exit(2);
    }

    let use_color = !cli.no_color;
    let options = FetchOptions::new(
        cli.timeout.get(),
        browser_cookie_access(&cli),
        cli.allow_custom_endpoints,
    );

    if let Some(interval) = cli.watch {
        let shutdown = tokio::signal::ctrl_c();
        tokio::pin!(shutdown);

        loop {
            if !cli.json {
                print!("\x1B[2J\x1B[H");
            }
            tokio::select! {
                signal = &mut shutdown => {
                    if let Err(error) = signal {
                        eprintln!("Could not listen for Ctrl+C: {error}");
                    }
                    break;
                }
                _ = run_once(&providers, use_color, cli.json, cli.pretty, &options) => {}
            }
            tokio::select! {
                signal = &mut shutdown => {
                    if let Err(error) = signal {
                        eprintln!("Could not listen for Ctrl+C: {error}");
                    }
                    break;
                }
                _ = tokio::time::sleep(Duration::from_secs(interval.get())) => {}
            }
        }
    } else {
        run_once(&providers, use_color, cli.json, cli.pretty, &options).await;
    }
}

async fn run_once(
    providers: &[crate::engine::snapshot::ProviderId],
    use_color: bool,
    json: bool,
    pretty: bool,
    options: &FetchOptions,
) {
    if json {
        let results = crate::engine::fetch_all(providers, options).await;
        output::json::json_print_results(&results, pretty);
    } else {
        let results = fetch_with_terminal_progress(providers, options, use_color).await;
        for result in &results {
            output::table::display_result(result, use_color);
        }
        println!();
    }
}

async fn fetch_with_terminal_progress(
    providers: &[crate::engine::snapshot::ProviderId],
    options: &FetchOptions,
    use_color: bool,
) -> Vec<ProviderResult> {
    let interactive = console::user_attended();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let total = providers.len();

    for provider in providers.iter().copied() {
        let tx = tx.clone();
        let options = options.clone();
        tokio::spawn(async move {
            let result = registry::fetch_provider(provider, &options).await;
            let _ = tx.send(result);
        });
    }
    drop(tx);

    let mut results: Vec<ProviderResult> = Vec::new();

    if interactive {
        let mut printed_lines = 0usize;
        while results.len() < total {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Some(result)) => {
                    results.push(result);
                    results.sort_by_key(|result| result.name.clone());
                    if printed_lines > 0 {
                        print!("\x1B[{printed_lines}A\x1B[J");
                    }
                    for result in &results {
                        output::table::display_result(result, use_color);
                    }
                    let pending = total - results.len();
                    for _ in 0..pending {
                        println!("\n  Loading...");
                    }
                    printed_lines = results
                        .iter()
                        .map(|result| 2 + result.windows.len())
                        .sum::<usize>()
                        + pending * 2;
                }
                Ok(None) => break,
                Err(_) => {}
            }
        }
        if printed_lines > 0 {
            print!("\x1B[{printed_lines}A\x1B[J");
        }
    } else {
        while results.len() < total {
            match rx.recv().await {
                Some(result) => results.push(result),
                None => break,
            }
        }
    }

    results.sort_by_key(|result| result.name.clone());
    results
}

fn browser_cookie_access(cli: &Cli) -> bool {
    cli.browser_cookies || !cli.no_browser_cookies
}

fn window_type_label(window_type: &WindowType) -> &'static str {
    match window_type {
        WindowType::FiveHours => "5h",
        WindowType::Weekly => "weekly",
        WindowType::Monthly => "monthly",
        WindowType::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_interval_must_be_non_zero() {
        assert!(Cli::try_parse_from(["remaining", "--watch", "0"]).is_err());
        assert!(Cli::try_parse_from(["remaining", "--watch", "1"]).is_ok());
    }

    #[test]
    fn pretty_output_requires_json() {
        assert!(Cli::try_parse_from(["remaining", "--pretty"]).is_err());
        assert!(Cli::try_parse_from(["remaining", "--json", "--pretty"]).is_ok());
    }

    #[test]
    fn timeout_must_be_non_zero() {
        assert!(Cli::try_parse_from(["remaining", "--timeout", "0"]).is_err());
        assert!(Cli::try_parse_from(["remaining", "--timeout", "1"]).is_ok());
    }

    #[test]
    fn browser_cookie_access_defaults_on_and_can_be_disabled() {
        let default = Cli::try_parse_from(["remaining"]).unwrap();
        assert!(browser_cookie_access(&default));

        let explicit = Cli::try_parse_from(["remaining", "--browser-cookies"]).unwrap();
        assert!(browser_cookie_access(&explicit));

        let disabled = Cli::try_parse_from(["remaining", "--no-browser-cookies"]).unwrap();
        assert!(!browser_cookie_access(&disabled));

        assert!(
            Cli::try_parse_from(["remaining", "--browser-cookies", "--no-browser-cookies"])
                .is_err()
        );
    }
}
