use crate::{
    commands::Command,
    crypto::{
        client_handshake, server_handshake, DecryptingReader, EncryptingWriter, NetworkMessage,
    },
    logger::{Level, LogMessage, Logger, LoggerPlusIterator},
    ui::{ChatMessage, Renderer, UI},
    Cli,
};
use clap::{crate_name, crate_version};
use std::{collections::HashMap, net::SocketAddr, str::FromStr};
use tokio::{
    io::WriteHalf,
    net::{TcpListener, TcpStream},
    runtime::Runtime,
    sync::mpsc,
};
use tokio_socks::tcp::Socks5Stream;
use tor_client_lib::{
    auth::TorAuthentication,
    control_connection::{OnionService, TorControlConnection},
    key::TorServiceId,
};
use x25519_dalek::{EphemeralSecret, PublicKey};

pub enum EngineEvent {
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

pub enum NetworkEvent {
    NewConnection(Connection),
    Message { sender: String, message: String },
    ConnectionClosed(Connection),
}

pub enum InputEvent {
    Message {
        recipient: Box<Connection>,
        message: String,
    },
    Command(Command),
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionDirection {
    Incoming,
    Outgoing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    tx: mpsc::Sender<EngineEvent>,
    log_level: Level,
}

impl TxLogger {
    fn new(tx: &mpsc::Sender<EngineEvent>, debug: bool) -> Self {
        Self {
            tx: tx.clone(),
            log_level: if debug { Level::Debug } else { Level::Info },
        }
    }
}

lazy_static::lazy_static! {
    static ref RUNTIME: Runtime = Runtime::new().unwrap();
}

impl Logger for TxLogger {
    fn log(&mut self, message: LogMessage) {
        if message.level >= self.log_level {
            RUNTIME.block_on(async move {
                self.tx
                    .send(EngineEvent::LogMessage(message))
                    .await
                    .unwrap();
            });
        }
    }

    fn set_log_level(&mut self, level: Level) {
        self.log_level = level;
    }
}

pub struct Engine {
    tor_control_connection: TorControlConnection,
    listener: TcpListener,
    config: Cli,
    writers: HashMap<SocketAddr, EncryptingWriter<WriteHalf<TcpStream>>>,
    onion_service: OnionService,
    id: TorServiceId,
    secret_key: EphemeralSecret,
    public_key: PublicKey,
    tx: mpsc::Sender<EngineEvent>,
    rx: mpsc::Receiver<EngineEvent>,
    debug: bool,
}

lazy_static::lazy_static! {
    static ref GREETING: Vec<String> = vec![
        "**************************************************************".to_string(),
        format!("              Welcome to {} version {}", crate_name!(), crate_version!()),
        "**************************************************************".to_string(),
        "Type ctrl-k to bring up a command window".to_string(),
        "Type 'help' in the command window to get a list of commands".to_string(),
        "Type ctrl-c anywhere, or 'quit' in the command window to exit".to_string(),
        String::new(),
    ];
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

        let listener = match TcpListener::bind(&format!("127.0.0.1:{}", cli.listen_port)).await {
            Ok(listener) => listener,
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "Error binding to port {}: {}",
                    cli.listen_port,
                    error
                ));
            }
        };

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        let id = onion_service.service_id.clone();

        // Generate a random X25519 keypair
        let secret = EphemeralSecret::random();
        let public = PublicKey::from(&secret);
        let debug = cli.debug;

        Ok(Engine {
            tor_control_connection: control_connection,
            listener,
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

    async fn handle_accept<L: Logger + ?Sized>(
        &mut self,
        stream: TcpStream,
        socket_addr: SocketAddr,
        logger: &mut L,
    ) -> Result<(), anyhow::Error> {
        let (reader, writer) = tokio::io::split(stream);

        let (reader, writer, peer_service_id) =
            server_handshake(reader, writer, &self.onion_service.signing_key, logger).await?;

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

    pub async fn get_network_event<L: Logger + ?Sized>(
        &mut self,
        logger: &mut L,
    ) -> Result<Option<NetworkEvent>, anyhow::Error> {
        tokio::select! {
            result = self.listener.accept() => {
                match result {
                    Ok((stream, socket_addr)) => {
                        let (reader, writer) = tokio::io::split(stream);

                        let (reader, writer, peer_service_id) =
                            server_handshake(reader, writer, &self.onion_service.signing_key, logger)
                                .await?;

                        let connection =
                            Connection::new(socket_addr, &peer_service_id, ConnectionDirection::Incoming);
                        self.writers.insert(socket_addr, writer);
                        let tx = self.tx.clone();
                        let debug = self.debug;
                        let cloned_connection = connection.clone();
                        tokio::spawn(async move {
                            Self::handle_connection(cloned_connection, reader, tx, debug).await;
                        });
                        Ok(Some(NetworkEvent::NewConnection(connection)))
                    }
                    Err(error) => Err(error)?,
                }
            },
            engine_event = self.rx.recv() => {
                match engine_event {
                    Some(EngineEvent::ClientConnection {
                        stream,
                        socket_addr,
                        id,
                    }) => {
                        // Setup the reader and writer
                        let (reader, writer) = tokio::io::split(stream);

                        let (reader, writer, peer_service_id) =
                            match client_handshake(reader, writer, &self.onion_service.signing_key, logger)
                                .await
                            {
                                Ok((reader, writer, peer_service_id)) => (reader, writer, peer_service_id),
                                Err(error) => {
                                    logger.log_error(&format!("Error in client handshake: {}", error));
                                    return Ok(None);
                                }
                            };

                        // Spawn the handler
                        let connection = Connection::new(socket_addr, &id, ConnectionDirection::Outgoing);
                        self.writers.insert(socket_addr, writer);
                        let tx = self.tx.clone();
                        let debug = self.debug;
                        let conn = connection.clone();
                        tokio::spawn(async move {
                            Self::handle_connection(conn, reader, tx, debug).await;
                        });
                        Ok(Some(NetworkEvent::NewConnection(connection)))
                    }
                    Some(EngineEvent::NewConnection(connection)) => {
                        if connection.direction == ConnectionDirection::Incoming {
                            logger.log_info(&format!(
                                "Got new connection from {}, id {}",
                                connection.address(),
                                connection.id().as_str()
                            ));
                        } else {
                            logger.log_info(&format!(
                                "Connected to {}, id {}",
                                connection.address(),
                                connection.id().as_str()
                            ));
                        }
                        Ok(Some(NetworkEvent::NewConnection(connection)))
                    }
                    Some(EngineEvent::Message { sender, message }) => {
                        Ok(Some(NetworkEvent::Message { sender, message }))
                    }
                    Some(EngineEvent::Error(error)) => {
                        logger.log_error(&format!("Got network error: {}", error));
                        Ok(None)
                    }
                    Some(EngineEvent::ConnectionClosed(connection)) => {
                        self.writers.remove(&connection.address);
                        logger.log_info(&format!("Lost connection to {}", connection.address));
                        Ok(Some(NetworkEvent::ConnectionClosed(connection)))
                    }
                    Some(EngineEvent::LogMessage(log_message)) => {
                        logger.log(log_message);
                        Ok(None)
                    }
                    None => panic!("Shouldn't be here"),
                }
            }
        }
    }

    pub async fn run(
        &mut self,
        renderer: &mut Renderer,
        ui: &mut dyn UI,
        logger: &mut dyn LoggerPlusIterator,
    ) -> Result<(), anyhow::Error> {
        if self.debug {
            logger.set_log_level(Level::Debug);
        }

        for line in GREETING.iter() {
            logger.log_info(line);
        }

        logger.log_info(&format!(
            "Onion service {} created",
            self.onion_service.address,
        ));

        loop {
            logger.log_debug("Calling render");
            ui.render(renderer, logger)?;

            tokio::select! {
                result = self.listener.accept() => {
                    match result {
                        Ok((stream, socket_addr)) => {
                            if let Err(error) = self.handle_accept(stream, socket_addr, logger).await {
                                logger.log_error(&format!("Error on accept: {}", error));
                            }
                        },
                        Err(error) => Err(error)?,
                    };
                },
                result = ui.get_input_event(logger) => {
                    match result {
                        Ok(input_event) => {
                            match input_event {
                                Some(InputEvent::Message { recipient, message }) => {
                                    let writer = self.writers.get_mut(&recipient.address).unwrap();
                                    let network_message = NetworkMessage::ChatMessage { sender: self.id.as_str().to_string(), message };
                                    if let Err(error) = writer.send(&network_message, logger).await {
                                        logger.log_error(&format!("Error sending message: {}", error));
                                    }
                                },
                                Some(InputEvent::Command(command)) => {
                                    match command {
                                        Command::Quit => break,
                                        _ => self.handle_command(logger, command).await,
                                    }
                                },
                                Some(InputEvent::Shutdown) => break,
                                None => {},
                            }
                        }
                        Err(error) => Err(error)?,
                    }
                },
                value = self.rx.recv() => {
                    if !self.handle_network_event(value, ui, logger).await {
                        break;
                    }
                },
            }
        }

        Ok(())
    }

    pub async fn handle_command<'a, L: Logger + ?Sized>(
        &mut self,
        logger: &mut L,
        command: Command,
    ) {
        match command {
            Command::Connect { address } => {
                if let Err(error) = self.connect(&address, logger).await {
                    logger.log_error(&format!("Connect error: {}", error));
                }
            }
            Command::Help { command } => Command::get_help(command, logger),
            Command::Quit => {}
        }
    }

    async fn handle_network_event<L: Logger + ?Sized>(
        &mut self,
        value: Option<EngineEvent>,
        ui: &mut dyn UI,
        logger: &mut L,
    ) -> bool {
        match value {
            Some(EngineEvent::ClientConnection {
                stream,
                socket_addr,
                id,
            }) => {
                // Setup the reader and writer
                let (reader, writer) = tokio::io::split(stream);

                let (reader, writer, peer_service_id) =
                    match client_handshake(reader, writer, &self.onion_service.signing_key, logger)
                        .await
                    {
                        Ok((reader, writer, peer_service_id)) => (reader, writer, peer_service_id),
                        Err(error) => {
                            logger.log_error(&format!("Error in client handshake: {}", error));
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
            Some(EngineEvent::NewConnection(connection)) => {
                if connection.direction == ConnectionDirection::Incoming {
                    logger.log_info(&format!(
                        "Got new connection from {}, id {}",
                        connection.address(),
                        connection.id().as_str()
                    ));
                } else {
                    logger.log_info(&format!(
                        "Connected to {}, id {}",
                        connection.address(),
                        connection.id().as_str()
                    ));
                }
                ui.add_chat(&connection);
            }
            Some(EngineEvent::Message { sender, message }) => {
                ui.add_message(ChatMessage::new(&sender, message));
            }
            Some(EngineEvent::Error(error)) => {
                logger.log_error(&format!("Got network error: {}", error));
            }
            Some(EngineEvent::ConnectionClosed(connection)) => {
                self.writers.remove(&connection.address);
                ui.remove_chat(&connection);
                logger.log_info(&format!("Lost connection to {}", connection.address));
            }
            Some(EngineEvent::LogMessage(log_message)) => {
                logger.log(log_message);
            }
            None => return false,
        }

        true
    }

    async fn handle_connection(
        connection: Connection,
        mut reader: DecryptingReader<tokio::io::ReadHalf<TcpStream>>,
        tx: mpsc::Sender<EngineEvent>,
        debug: bool,
    ) {
        let _ = tx
            .send(EngineEvent::NewConnection(connection.clone()))
            .await;
        let mut logger = TxLogger::new(&tx, debug);
        loop {
            match reader.read(&mut logger).await {
                Ok(Some(message)) => match message {
                    NetworkMessage::ChatMessage { sender, message } => {
                        let _ = tx.send(EngineEvent::Message { sender, message }).await;
                    }
                    NetworkMessage::ServiceIdMessage { .. } => {
                        let _ = tx
                            .send(EngineEvent::Error(anyhow::anyhow!(
                                "Unexpected ServiceID message received"
                            )))
                            .await;
                    }
                },
                Ok(None) => {
                    let _ = tx.send(EngineEvent::ConnectionClosed(connection)).await;
                    break;
                }
                Err(error) => {
                    let _ = tx
                        .send(EngineEvent::Error(anyhow::anyhow!(
                            "Error reading from connection: {}",
                            error
                        )))
                        .await;
                    let _ = tx.send(EngineEvent::ConnectionClosed(connection)).await;
                    break;
                }
            }
        }
    }

    pub async fn connect<L: Logger + ?Sized>(
        &mut self,
        address: &str,
        logger: &mut L,
    ) -> Result<(), anyhow::Error> {
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

        logger.log_info(&format!("Connecting to {}...", address));

        let tx = self.tx.clone();
        let address = address.to_string();
        tokio::spawn(async move {
            // Connect through the Tor SOCKS proxy
            let stream = match Socks5Stream::connect(socket_addr, address).await {
                Ok(stream) => stream.into_inner(),
                Err(error) => {
                    let _ = tx.send(EngineEvent::Error(error.into())).await;
                    return;
                }
            };

            // Let the main thread know we're connected
            let _ = tx
                .send(EngineEvent::ClientConnection {
                    stream,
                    socket_addr,
                    id,
                })
                .await;
        });

        Ok(())
    }
}
