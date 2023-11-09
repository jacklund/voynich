use crate::app::App;
use crate::engine::Engine;
use crate::logger::{Level, Logger, StandardLogger};
use clap::Parser;
use tokio::net::TcpListener;

mod app;
mod app_context;
mod chat;
mod commands;
mod crypto;
mod engine;
mod input;
mod logger;
mod root;
mod term;
mod theme;

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
async fn main() {
    let cli = Cli::parse();

    let mut logger = StandardLogger::new(500);
    if cli.debug {
        logger.set_log_level(Level::Debug);
    }

    let listener = match TcpListener::bind(&format!("127.0.0.1:{}", cli.listen_port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Error binding to port {}: {}", cli.listen_port, error);
            return;
        }
    };

    let mut engine = match Engine::new(cli).await {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("Error creating engine: {}", error);
            return;
        }
    };
    let _ = App::run(&mut engine, &listener, &mut logger).await;
}
