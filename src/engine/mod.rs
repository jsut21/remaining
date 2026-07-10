pub mod registry;
pub mod snapshot;

use snapshot::ProviderResult;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct FetchOptions {
    pub timeout: Arc<Duration>,
    pub allow_browser_cookies: bool,
    pub allow_custom_endpoints: bool,
}

impl FetchOptions {
    pub fn new(
        timeout_secs: u64,
        allow_browser_cookies: bool,
        allow_custom_endpoints: bool,
    ) -> Self {
        Self {
            timeout: Arc::new(Duration::from_secs(timeout_secs)),
            allow_browser_cookies,
            allow_custom_endpoints,
        }
    }
}

pub async fn fetch_all(
    providers: &[snapshot::ProviderId],
    options: &FetchOptions,
) -> Vec<ProviderResult> {
    let handles: Vec<_> = providers
        .iter()
        .copied()
        .map(|provider| {
            let options = options.clone();
            tokio::spawn(async move { registry::fetch_provider(provider, &options).await })
        })
        .collect();

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(error) => eprintln!("Error fetching provider: {error}"),
        }
    }
    results.sort_by_key(|result| result.name.clone());
    results
}
