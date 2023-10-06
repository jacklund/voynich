use crate::ui::{ChatMessage, InputEvent, Renderer, UI};
use crate::{Cli, TermInputStream};
use anyhow;
use futures::{SinkExt, StreamExt};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::str::FromStr;
use tokio;
use tokio::net::{TcpListener, TcpStream};
use tokio_socks::tcp::Socks5Stream;
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec, LinesCodecError};
use tor_client_lib::{
    auth::TorAuthentication,
    control_connection::{OnionService, TorControlConnection},
};

enum NetworkEvent {
    NewConnection(Connection),
    Message { sender: String, message: String },
    Error(std::io::Error),
    ConnectionClosed(Connection),
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Connection {
    address: SocketAddr,
    id: String,
}

impl Connection {
    fn new(address: SocketAddr, id: &str) -> Self {
        Self {
            address,
            id: id.to_string(),
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

pub struct Engine {
    tor_control_connection: TorControlConnection,
    config: Cli,
    writers: HashMap<SocketAddr, tokio::io::WriteHalf<TcpStream>>,
    onion_service: OnionService,
    id: String,
    tx: tokio::sync::mpsc::Sender<NetworkEvent>,
    rx: tokio::sync::mpsc::Receiver<NetworkEvent>,
}

impl Engine {
    pub async fn new(cli: Cli) -> Result<Self, anyhow::Error> {
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

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        let id = onion_service.service_id.clone();

        Ok(Engine {
            tor_control_connection: control_connection,
            config: cli,
            writers: HashMap::new(),
            onion_service,
            id,
            tx,
            rx,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub async fn run(
        &mut self,
        mut input_stream: TermInputStream,
        listener: &TcpListener,
        renderer: &mut Renderer,
        ui: &mut UI,
    ) -> Result<(), anyhow::Error> {
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
                            let mut framed_reader = FramedRead::new(reader, LinesCodec::new());
                            let id = match framed_reader.next().await {
                                Some(result) => result?,
                                None => continue,
                            };
                            let connection = Connection::new(socket_addr, &id);
                            self.writers.insert(socket_addr, writer);
                            let tx = self.tx.clone();
                            tokio::spawn(async move {
                                Self::handle_connection(connection, framed_reader, tx).await;
                            });
                        },
                        Err(error) => Err(error)?,
                    };
                },
                result = input_stream.select_next_some() => {
                    match result {
                        Ok(event) => match ui.handle_input_event(self, event).await? {
                            Some(InputEvent::Message { recipient, message }) => {
                                let writer = self.writers.get_mut(&recipient.address).unwrap();
                                let mut framed_writer = FramedWrite::new(writer, LinesCodec::new());
                                if let Err(error) = framed_writer.send(message).await {
                                    ui.log_error(&format!("Error sending message: {}", error));
                                }
                            },
                            Some(InputEvent::Shutdown) => break,
                            None => {},
                        },
                        Err(error) => Err(error)?,
                    }
                },
                value = self.rx.recv() => {
                    match value {
                        Some(NetworkEvent::NewConnection(connection)) => {
                            ui.log_info(&format!("Got new connection from {}, id {}", connection.address(), connection.id()));
                            ui.add_chat(&connection);
                        }
                        Some(NetworkEvent::Message { sender, message }) => {
                            ui.add_message(ChatMessage::new(&sender, message));
                        },
                        Some(NetworkEvent::Error(error)) => {
                            ui.log_error(&format!("Got network error: {}", error));
                        },
                        Some(NetworkEvent::ConnectionClosed(connection)) => {
                            self.writers.remove(&connection.address);
                            ui.remove_chat(&connection);
                            ui.log_info(&format!("Lost connection to {}", connection.address));
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
        ui: &mut UI,
        mut command_args: VecDeque<&'a str>,
    ) -> Result<Option<InputEvent>, anyhow::Error> {
        let command = command_args.pop_front();
        if let Some(command) = command {
            match command.to_ascii_lowercase().as_str() {
                "connect" => {
                    match command_args.pop_front() {
                        Some(address) => self.connect(address).await?,
                        None => {
                            ui.log_error("'connect' command requires a tor address to connect to");
                        }
                    }
                    Ok(None)
                }
                "quit" => Ok(Some(InputEvent::Shutdown)),
                _ => {
                    ui.log_error(&format!("Unknown command '{}'", command));
                    Ok(None)
                }
            }
        } else {
            Ok(None)
        }
    }

    async fn handle_connection(
        connection: Connection,
        mut framed_reader: FramedRead<tokio::io::ReadHalf<TcpStream>, LinesCodec>,
        tx: tokio::sync::mpsc::Sender<NetworkEvent>,
    ) {
        let _ = tx
            .send(NetworkEvent::NewConnection(connection.clone()))
            .await;
        loop {
            match framed_reader.next().await {
                Some(Ok(line)) => {
                    let _ = tx
                        .send(NetworkEvent::Message {
                            sender: connection.id.clone(),
                            message: line,
                        })
                        .await;
                }
                Some(Err(error)) => match error {
                    LinesCodecError::MaxLineLengthExceeded => {
                        let _ = tx
                            .send(NetworkEvent::Error(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                "Maximum line length exceeded",
                            )))
                            .await;
                    }
                    LinesCodecError::Io(error) => {
                        let _ = tx.send(NetworkEvent::Error(error)).await;
                    }
                },
                None => {
                    let _ = tx.send(NetworkEvent::ConnectionClosed(connection)).await;
                    break;
                }
            }
        }
    }

    pub async fn connect(&mut self, address: &str) -> Result<(), anyhow::Error> {
        // Use the proxy address for our socket address
        let socket_addr = SocketAddr::from_str(&self.config.tor_proxy_address)?;

        // Parse the address to get the ID
        let mut iter = address.rsplitn(2, ':');
        let _port: u16 = iter
            .next()
            .and_then(|port_str| port_str.parse().ok())
            .ok_or(anyhow::anyhow!("Invalid port value"))?;
        let domain = iter.next().ok_or(anyhow::anyhow!("Invalid domain"))?;
        let id = domain.split('.').collect::<Vec<&str>>()[0];

        // Connect through the Tor SOCKS proxy
        let stream = Socks5Stream::connect(socket_addr, address)
            .await?
            .into_inner();

        // Setup the reader and writer
        let (reader, writer) = tokio::io::split(stream);
        let framed_reader = FramedRead::new(reader, LinesCodec::new());
        let mut framed_writer = FramedWrite::new(writer, LinesCodec::new());

        // Send the ID
        framed_writer
            .send(self.onion_service.service_id.clone())
            .await?;

        // Spawn the handler
        let writer = framed_writer.into_inner();
        let connection = Connection::new(socket_addr, &id);
        self.writers.insert(socket_addr, writer);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            Self::handle_connection(connection, framed_reader, tx).await;
        });

        Ok(())
    }
}
