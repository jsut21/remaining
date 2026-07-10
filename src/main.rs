mod cli;
mod engine;
mod output;
mod providers;
mod utils;

#[tokio::main]
async fn main() {
    cli::run().await;
}
