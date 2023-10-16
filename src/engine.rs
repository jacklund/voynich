use crate::{
    crypto::{
        client_handshake, server_handshake, DecryptingReader, EncryptingWriter, NetworkMessage,
    },
    logger::{Level, LogMessage, Logger},
    ui::{ChatMessage, InputEvent, Renderer, UI},
    Cli, TermInputStream,
};
use async_trait::async_trait;
use futures::StreamExt;
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    str::FromStr,
};
use tokio::{
    io::WriteHalf,
    net::{TcpListener, TcpStream},
    sync::mpsc,
};
use tokio_socks::tcp::Socks5Stream;
use tor_client_lib::{
    auth::TorAuthentication,
    control_connection::{OnionService, TorControlConnection},
    key::TorServiceId,
};
use x25519_dalek::{EphemeralSecret, PublicKey};

pub enum NetworkEvent {
    ClientConnection {
        stream: TcpStream,
        socket_addr: SocketAddr,
        id: TorServiceId,
    },
    NewConnection(Connection),
    Message {
        sender: String,
        message: String,
    },
    Error(anyhow::Error),
    ConnectionClosed(Connection),
    LogMessage(LogMessage),
}

#[derive(Clone, PartialEq, Eq)]
pub enum ConnectionDirection {
    Incoming,
    Outgoing,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Connection {
    address: SocketAddr,
    id: TorServiceId,
    direction: ConnectionDirection,
}

impl Connection {
    fn new(address: SocketAddr, id: &TorServiceId, direction: ConnectionDirection) -> Self {
        Self {
            address,
            id: id.clone(),
            direction,
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn id(&self) -> TorServiceId {
        self.id.clone()
    }
}

struct TxLogger {
    tx: mpsc::Sender<NetworkEvent>,
    log_level: Level,
}

impl TxLogger {
    fn new(tx: &mpsc::Sender<NetworkEvent>, debug: bool) -> Self {
        Self {
            tx: tx.clone(),
            log_level: if debug { Level::Debug } else { Level::Info },
        }
    }
}

#[async_trait]
impl Logger for TxLogger {
    async fn log(&mut self, message: LogMessage) {
        if message.level >= self.log_level {
            self.tx
                .send(NetworkEvent::LogMessage(message))
                .await
                .unwrap();
        }
    }

    fn set_log_level(&mut self, level: Level) {
        self.log_level = level;
    }
}

pub struct Engine {
    tor_control_connection: TorControlConnection,
    config: Cli,
    writers: HashMap<SocketAddr, EncryptingWriter<WriteHalf<TcpStream>>>,
    onion_service: OnionService,
    id: TorServiceId,
    secret_key: EphemeralSecret,
    public_key: PublicKey,
    tx: mpsc::Sender<NetworkEvent>,
    rx: mpsc::Receiver<NetworkEvent>,
    debug: bool,
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

        // Generate a random X25519 keypair
        let secret = EphemeralSecret::random();
        let public = PublicKey::from(&secret);
        let debug = cli.debug;

        Ok(Engine {
            tor_control_connection: control_connection,
            config: cli,
            writers: HashMap::new(),
            onion_service,
            id,
            secret_key: secret,
            public_key: public,
            tx,
            rx,
            debug,
        })
    }

    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    async fn handle_accept(
        &mut self,
        stream: TcpStream,
        socket_addr: SocketAddr,
        ui: &mut UI,
    ) -> Result<(), anyhow::Error> {
        let (reader, writer) = tokio::io::split(stream);

        let (reader, writer, peer_service_id) =
            server_handshake(reader, writer, &self.onion_service.signing_key, ui).await?;

        let connection =
            Connection::new(socket_addr, &peer_service_id, ConnectionDirection::Incoming);
        self.writers.insert(socket_addr, writer);
        let tx = self.tx.clone();
        let debug = self.debug;
        tokio::spawn(async move {
            Self::handle_connection(connection, reader, tx, debug).await;
        });

        Ok(())
    }

    pub async fn run(
        &mut self,
        mut input_stream: TermInputStream,
        listener: &TcpListener,
        renderer: &mut Renderer,
        ui: &mut UI,
    ) -> Result<(), anyhow::Error> {
        if self.debug {
            ui.set_log_level(Level::Debug);
        }

        ui.log_info(&format!(
            "Onion service {} created",
            self.onion_service.address,
        ))
        .await;

        loop {
            renderer.render(ui)?;

            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, socket_addr)) => {
                            if let Err(error) = self.handle_accept(stream, socket_addr, ui).await {
                                ui.log_error(&format!("Error on accept: {}", error)).await;
                            }
                        },
                        Err(error) => Err(error)?,
                    };
                },
                result = input_stream.select_next_some() => {
                    match result {
                        Ok(event) => match ui.handle_input_event(self, event).await? {
                            Some(InputEvent::Message { recipient, message }) => {
                                let writer = self.writers.get_mut(&recipient.address).unwrap();
                                let network_message = NetworkMessage::ChatMessage { sender: self.id.as_str().to_string(), message };
                                if let Err(error) = writer.send(&network_message, ui).await {
                                    ui.log_error(&format!("Error sending message: {}", error)).await;
                                }
                            },
                            Some(InputEvent::Shutdown) => break,
                            None => {},
                        },
                        Err(error) => Err(error)?,
                    }
                },
                value = self.rx.recv() => {
                    if !self.handle_network_event(value, ui).await {
                        break;
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
                        Some(address) => self.connect(address, ui).await?,
                        None => {
                            ui.log_error("'connect' command requires a tor address to connect to")
                                .await;
                        }
                    }
                    Ok(None)
                }
                "quit" => Ok(Some(InputEvent::Shutdown)),
                _ => {
                    ui.log_error(&format!("Unknown command '{}'", command))
                        .await;
                    Ok(None)
                }
            }
        } else {
            Ok(None)
        }
    }

    async fn handle_network_event(&mut self, value: Option<NetworkEvent>, ui: &mut UI) -> bool {
        match value {
            Some(NetworkEvent::ClientConnection {
                stream,
                socket_addr,
                id,
            }) => {
                // Setup the reader and writer
                let (reader, writer) = tokio::io::split(stream);

                let (reader, writer, peer_service_id) =
                    match client_handshake(reader, writer, &self.onion_service.signing_key, ui)
                        .await
                    {
                        Ok((reader, writer, peer_service_id)) => (reader, writer, peer_service_id),
                        Err(error) => {
                            ui.log_error(&format!("Error in client handshake: {}", error))
                                .await;
                            return true;
                        }
                    };

                // Spawn the handler
                let connection = Connection::new(socket_addr, &id, ConnectionDirection::Outgoing);
                self.writers.insert(socket_addr, writer);
                let tx = self.tx.clone();
                let debug = self.debug;
                tokio::spawn(async move {
                    Self::handle_connection(connection, reader, tx, debug).await;
                });
            }
            Some(NetworkEvent::NewConnection(connection)) => {
                if connection.direction == ConnectionDirection::Incoming {
                    ui.log_info(&format!(
                        "Got new connection from {}, id {}",
                        connection.address(),
                        connection.id().as_str()
                    ))
                    .await;
                } else {
                    ui.log_info(&format!(
                        "Connected to {}, id {}",
                        connection.address(),
                        connection.id().as_str()
                    ))
                    .await;
                }
                ui.add_chat(&connection);
            }
            Some(NetworkEvent::Message { sender, message }) => {
                ui.add_message(ChatMessage::new(&sender, message));
            }
            Some(NetworkEvent::Error(error)) => {
                ui.log_error(&format!("Got network error: {}", error)).await;
            }
            Some(NetworkEvent::ConnectionClosed(connection)) => {
                self.writers.remove(&connection.address);
                ui.remove_chat(&connection);
                ui.log_info(&format!("Lost connection to {}", connection.address))
                    .await;
            }
            Some(NetworkEvent::LogMessage(log_message)) => {
                ui.log(log_message).await;
            }
            None => return false,
        }

        true
    }

    async fn handle_connection(
        connection: Connection,
        mut reader: DecryptingReader<tokio::io::ReadHalf<TcpStream>>,
        tx: mpsc::Sender<NetworkEvent>,
        debug: bool,
    ) {
        let _ = tx
            .send(NetworkEvent::NewConnection(connection.clone()))
            .await;
        let mut logger = TxLogger::new(&tx, debug);
        loop {
            match reader.read(&mut logger).await {
                Ok(Some(message)) => match message {
                    NetworkMessage::ChatMessage { sender, message } => {
                        let _ = tx.send(NetworkEvent::Message { sender, message }).await;
                    }
                    NetworkMessage::ServiceIdMessage { .. } => {
                        let _ = tx
                            .send(NetworkEvent::Error(anyhow::anyhow!(
                                "Unexpected ServiceID message received"
                            )))
                            .await;
                    }
                },
                Ok(None) => {
                    let _ = tx.send(NetworkEvent::ConnectionClosed(connection)).await;
                    break;
                }
                Err(error) => {
                    let _ = tx
                        .send(NetworkEvent::Error(anyhow::anyhow!(
                            "Error reading from connection: {}",
                            error
                        )))
                        .await;
                    let _ = tx.send(NetworkEvent::ConnectionClosed(connection)).await;
                    break;
                }
            }
        }
    }

    pub async fn connect(&mut self, address: &str, ui: &mut UI) -> Result<(), anyhow::Error> {
        // Use the proxy address for our socket address
        let socket_addr = SocketAddr::from_str(&self.config.tor_proxy_address)?;

        // Parse the address to get the ID
        let mut iter = address.rsplitn(2, ':');
        let _port: u16 = iter
            .next()
            .and_then(|port_str| port_str.parse().ok())
            .ok_or(anyhow::anyhow!("Invalid port value"))?;
        let domain = iter.next().ok_or(anyhow::anyhow!("Invalid domain"))?;
        let id = TorServiceId::from_str(domain.split('.').collect::<Vec<&str>>()[0])?;

        ui.log_info(&format!("Connecting to {}...", address)).await;

        let tx = self.tx.clone();
        let address = address.to_string();
        tokio::spawn(async move {
            // Connect through the Tor SOCKS proxy
            let stream = match Socks5Stream::connect(socket_addr, address).await {
                Ok(stream) => stream.into_inner(),
                Err(error) => {
                    let _ = tx.send(NetworkEvent::Error(error.into())).await;
                    return;
                }
            };

            // Let the main thread know we're connected
            let _ = tx
                .send(NetworkEvent::ClientConnection {
                    stream,
                    socket_addr,
                    id,
                })
                .await;
        });

        Ok(())
    }
}
