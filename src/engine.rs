use crate::ui::{InputEvent, Renderer, UI};
use crate::{Cli, TermInputStream};
use anyhow::anyhow;
use futures::stream::StreamExt;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use tokio;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};
use tor_client_lib::{
    auth::TorAuthentication,
    control_connection::{OnionService, TorControlConnection},
};

#[derive(Clone, Debug, Default)]
pub struct EngineConfig {
    pub tor_proxy_address: Option<SocketAddr>,
    pub tor_control_address: Option<SocketAddr>,
}

enum NetworkEvent {
    NewConnection(SocketAddr),
    Message {
        address: SocketAddr,
        message: String,
    },
    Error(std::io::Error),
    ConnectionClosed(SocketAddr),
}

pub struct Engine {
    tor_control_connection: TorControlConnection,
    config: Cli,
    writers: HashMap<SocketAddr, tokio::io::WriteHalf<TcpStream>>,
    onion_service: OnionService,
}

impl Engine {
    pub async fn new(cli: Cli) -> Result<Self, Box<dyn std::error::Error>> {
        let mut control_connection = TorControlConnection::connect(cli.tor_address.clone()).await?;
        control_connection
            .authenticate(TorAuthentication::SafeCookie(None))
            .await?;

        let onion_service = control_connection
            .create_onion_service(
                cli.listen_port,
                &format!("localhost:{}", cli.listen_port),
                cli.transient,
                None,
            )
            .await?;

        Ok(Engine {
            tor_control_connection: control_connection,
            config: cli,
            writers: HashMap::new(),
            onion_service,
        })
    }

    pub async fn run(
        &mut self,
        mut input_stream: TermInputStream,
        listener: &TcpListener,
        renderer: &mut Renderer,
        ui: &mut UI,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);

        ui.log_info(&format!(
            "Onion service {} created",
            self.onion_service.address,
        ));

        loop {
            renderer.render(ui)?;

            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, socket_addr)) => {
                            let (reader, writer) = tokio::io::split(stream);
                            self.writers.insert(socket_addr, writer);
                            let tx = tx.clone();
                            tokio::spawn(async move {
                                let _ = tx.send(NetworkEvent::NewConnection(socket_addr)).await;
                                let mut reader = FramedRead::new(reader, LinesCodec::new());
                                loop {
                                    match reader.next().await {
                                        Some(Ok(line)) => {
                                            let _ = tx.send(NetworkEvent::Message { address: socket_addr, message: line }).await;
                                        },
                                        Some(Err(error)) => match error {
                                            LinesCodecError::MaxLineLengthExceeded => {
                                                let _ = tx.send(NetworkEvent::Error(std::io::Error::new(std::io::ErrorKind::Other, "Maximum line length exceeded"))).await;
                                            },
                                            LinesCodecError::Io(error) => {
                                                let _ = tx.send(NetworkEvent::Error(error)).await;
                                            },
                                        },
                                        None => {
                                            let _ = tx.send(NetworkEvent::ConnectionClosed(socket_addr)).await;
                                            break;
                                        },
                                    }
                                }
                            });
                        },
                        Err(error) => Err(error)?,
                    };
                },
                result = input_stream.select_next_some() => {
                    match result {
                        Ok(event) => match ui.handle_input_event(self, event).await? {
                            Some(InputEvent::Message { sender: _, message: _ }) => {},
                            Some(InputEvent::Shutdown) => break,
                            None => {},
                        },
                        Err(error) => Err(error)?,
                    }
                },
                value = rx.recv() => {
                    match value {
                        Some(NetworkEvent::NewConnection(address)) => ui.log_info(&format!("Got new connection from {}", address)),
                        Some(NetworkEvent::Message { address, message }) => {
                            ui.log_info(&format!("New message from {}: {}", address, message));
                        },
                        Some(NetworkEvent::Error(error)) => {
                            ui.log_error(&format!("Got network error: {}", error));
                        },
                        Some(NetworkEvent::ConnectionClosed(address)) => {
                            self.writers.remove(&address);
                            ui.log_info(&format!("Lost connection to {}", address));
                        },
                        None => break,
                    }
                },
            }
        }

        Ok(())
    }

    pub async fn handle_command<'a>(
        &mut self,
        mut _command_args: VecDeque<&'a str>,
    ) -> Result<Option<InputEvent>, Box<dyn std::error::Error>> {
        unimplemented!()
    }
}
