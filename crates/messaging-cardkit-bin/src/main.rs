use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = messaging_cardkit_bin::Cli::parse();
    messaging_cardkit_bin::run(cli).await
}
