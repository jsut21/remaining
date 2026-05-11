mod types;
mod utils;
mod display;
mod json_output;
mod providers;

use clap::Parser;
use std::sync::Arc;
use std::time::Duration;
use types::{ProviderId, ProviderResult};

#[derive(Parser)]
#[command(name = "remaining")]
#[command(about = "Check remaining usage quotas for Z.ai, Codex, OpenCode, and Ollama Cloud")]
struct Cli {
    #[arg(long, help = "Output in JSON format")]
    json: bool,

    #[arg(long, help = "Pretty-print JSON output")]
    pretty: bool,

    #[arg(long, value_delimiter = ',', help = "Only check specific providers (zai,codex,opencode,ollama)")]
    provider: Vec<String>,

    #[arg(long, help = "Disable colored output")]
    no_color: bool,

    #[arg(long, help = "Auto-refresh every N seconds")]
    watch: Option<u64>,

    #[arg(long, default_value = "15", help = "Request timeout in seconds")]
    timeout: u64,
}

fn parse_providers(spec: &[String]) -> Vec<ProviderId> {
    if spec.is_empty() {
        vec![ProviderId::Zai, ProviderId::Codex, ProviderId::OpenCode, ProviderId::Ollama]
    } else {
        spec.iter()
            .filter_map(|p| match p.as_str() {
                "zai" => Some(ProviderId::Zai),
                "codex" => Some(ProviderId::Codex),
                "opencode" => Some(ProviderId::OpenCode),
                "ollama" => Some(ProviderId::Ollama),
                _ => None,
            })
            .collect()
    }
}

async fn run_once(providers: &[ProviderId], use_color: bool, json: bool, pretty: bool, timeout_secs: u64) {
    let timeout = Arc::new(Duration::from_secs(timeout_secs));

    if json {
        let handles: Vec<_> = providers.iter().map(|p| {
            let p = *p;
            let timeout = Arc::clone(&timeout);
            tokio::spawn(async move {
                match p {
                    ProviderId::Zai => providers::zai::fetch(&timeout).await,
                    ProviderId::Codex => providers::codex::fetch(&timeout).await,
                    ProviderId::OpenCode => providers::opencode::fetch(&timeout).await,
                    ProviderId::Ollama => providers::ollama::fetch(&timeout).await,
                }
            })
        }).collect();

        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => eprintln!("Error fetching provider: {}", e),
            }
        }
        results.sort_by_key(|r| r.name.clone());
        json_output::json_print_results(&results, pretty);
    } else {
        let interactive = console::user_attended();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let total = providers.len();

        for p in providers.iter().copied() {
            let tx = tx.clone();
            let timeout = Arc::clone(&timeout);
            tokio::spawn(async move {
                let result = match p {
                    ProviderId::Zai => providers::zai::fetch(&timeout).await,
                    ProviderId::Codex => providers::codex::fetch(&timeout).await,
                    ProviderId::OpenCode => providers::opencode::fetch(&timeout).await,
                    ProviderId::Ollama => providers::ollama::fetch(&timeout).await,
                };
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
                        results.sort_by_key(|r| r.name.clone());
                        if printed_lines > 0 {
                            print!("\x1B[{}A\x1B[J", printed_lines);
                        }
                        for r in &results {
                            display::display_result(r, use_color);
                        }
                        let pending = total - results.len();
                        for _ in 0..pending {
                            println!("\n  ⠋ Loading...");
                        }
                        printed_lines = results.iter().map(|r| 2 + r.windows.len()).sum::<usize>() + pending * 2;
                    }
                    Ok(None) => break,
                    Err(_) => {}
                }
            }
            if printed_lines > 0 {
                print!("\x1B[{}A\x1B[J", printed_lines);
            }
        } else {
            while results.len() < total {
                match rx.recv().await {
                    Some(result) => results.push(result),
                    None => break,
                }
            }
        }

        results.sort_by_key(|r| r.name.clone());
        for result in &results {
            display::display_result(result, use_color);
        }
        println!();
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let providers = parse_providers(&cli.provider);
    let use_color = !cli.no_color;
    let timeout_secs = cli.timeout;

    if let Some(interval) = cli.watch {
        let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let r = Arc::clone(&running);
        let _ = ctrlc::set_handler(move || { r.store(false, std::sync::atomic::Ordering::Relaxed); });
        loop {
            if !running.load(std::sync::atomic::Ordering::Relaxed) { break; }
            if !cli.json {
                print!("\x1B[2J\x1B[H");
            }
            run_once(&providers, use_color, cli.json, cli.pretty, timeout_secs).await;
            std::thread::sleep(Duration::from_secs(interval));
        }
    } else {
        run_once(&providers, use_color, cli.json, cli.pretty, timeout_secs).await;
    }
}