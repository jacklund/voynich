use clap::Parser;
use crossterm::event::{Event as TermEvent, EventStream};
use futures::task::Poll;
// use futures::StreamExt;
use crate::engine::Engine;
use crate::ui::{Renderer, UI};
use futures_lite::stream::StreamExt;
use std::pin::Pin;
use std::task::Context;
use tokio::net::TcpListener;
use tor_client_lib::{auth::TorAuthentication, control_connection::TorControlConnection};

mod engine;
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

    /// Turn debugging information on
    #[arg(short, long, default_value_t = true)]
    transient: bool,
}

pub struct TermInputStream {
    reader: EventStream,
}

impl TermInputStream {
    fn new() -> Self {
        Self {
            reader: EventStream::new(),
        }
    }
}

impl futures::stream::Stream for TermInputStream {
    type Item = Result<TermEvent, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.reader.poll_next(cx)
    }
}

impl futures::stream::FusedStream for TermInputStream {
    fn is_terminated(&self) -> bool {
        false
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let listener = TcpListener::bind(&format!("127.0.0.1:{}", cli.listen_port)).await?;
    let mut renderer = Renderer::new();
    let mut ui = UI::new();

    let mut engine = Engine::new(cli).await?;
    engine
        .run(TermInputStream::new(), &listener, &mut renderer, &mut ui)
        .await?;

    Ok(())
}
