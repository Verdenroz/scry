mod cli;
mod client;
mod commands;
mod output;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::dispatch(&args).await {
        Ok(()) => Ok(()),
        Err(error) => {
            eprintln!("scry: {error}");
            std::process::exit(1);
        }
    }
}
