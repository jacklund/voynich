use crate::app::App;
use crate::engine::Engine;
use crate::logger::StandardLogger;
use crate::ui::{Renderer, TerminalUI};
use clap::Parser;

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
async fn main() {
    let cli = Cli::parse();

    let mut renderer = Renderer::new();
    let mut logger = StandardLogger::new(500);

    let mut engine = match Engine::new(cli).await {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("Error creating engine: {}", error);
            return;
        }
    };
    let _ = App::run(&mut engine, &mut logger).await;
    // let mut ui = TerminalUI::new(engine.id().as_str());
    // if let Err(error) = engine.run(&mut renderer, &mut ui, &mut logger).await {
    //     eprintln!("Error: {}", error);
    // }
}
