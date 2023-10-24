use crate::engine::Engine;
use crate::logger::StandardLogger;
use crate::ui::{Renderer, UI};
use clap::Parser;
use tokio::net::TcpListener;

mod crypto;
mod engine;
mod logger;
mod ui;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Tor control address
    #[arg(long, value_name = "ADDRESS", default_value_t = String::from("127.0.0.1:9051"))]
    tor_address: String,

    /// Tor proxy address
    #[arg(long, value_name = "ADDRESS", default_value_t = String::from("127.0.0.1:9050"))]
    tor_proxy_address: String,

    /// Listen on port
    #[arg(short, long, value_name = "PORT")]
    listen_port: u16,

    /// Use transient onion service
    #[arg(short, long, default_value_t = true)]
    transient: bool,

    /// Use debug logging
    #[arg(short, long, default_value_t = false)]
    debug: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let listener = TcpListener::bind(&format!("127.0.0.1:{}", cli.listen_port)).await?;
    let mut renderer = Renderer::new();
    let mut logger = StandardLogger::new(500);

    let mut engine = Engine::new(cli).await?;
    let mut ui = UI::new(engine.id());
    engine
        .run(&listener, &mut renderer, &mut ui, &mut logger)
        .await?;

    Ok(())
}
