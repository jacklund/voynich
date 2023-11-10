use crate::{
    chat::ChatMessage,
    crypto::{
        client_handshake, server_handshake, DecryptingReader, EncryptingWriter, NetworkMessage,
        ServiceIdMessage,
    },
    logger::{Level, LogMessage, Logger, StandardLogger},
    Cli,
};
use std::{collections::HashMap, net::SocketAddr, str::FromStr};
use tokio::{net::TcpStream, sync::mpsc};
use tokio_socks::tcp::Socks5Stream;
use tor_client_lib::{
    auth::TorAuthentication,
    control_connection::{OnionService, TorControlConnection},
    TorServiceId,
};

enum EngineEvent {
    NewConnection(Box<Connection>, mpsc::Sender<ConnectionEvent>),
    Message(Box<ChatMessage>),
    Error(anyhow::Error),
    ConnectionClosed(Box<Connection>),
    LogMessage(LogMessage),
}

enum ConnectionEvent {
    Message(Box<ChatMessage>),
    CloseConnection,
}

pub enum NetworkEvent {
    NewConnection(Box<Connection>),
    Message(Box<ChatMessage>),
    ConnectionClosed(Box<Connection>),
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

    pub fn id(&self) -> TorServiceId {
        self.id.clone()
    }
}

pub struct TxLogger {
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

    async fn log(&mut self, message: LogMessage) {
        if message.level >= self.log_level {
            self.tx
                .send(EngineEvent::LogMessage(message))
                .await
                .unwrap();
        }
    }

    async fn log_message(&mut self, level: Level, message: String) {
        self.log(LogMessage::new(level, &message)).await;
    }

    pub async fn log_error(&mut self, message: &str) {
        self.log_message(Level::Error, format!("ERROR: {}", message))
            .await;
    }

    pub async fn log_info(&mut self, message: &str) {
        self.log_message(Level::Info, format!("INFO: {}", message))
            .await;
    }

    pub async fn log_debug(&mut self, message: &str) {
        self.log_message(Level::Debug, format!("DEBUG: {}", message))
            .await;
    }
}

pub struct Engine {
    config: Cli,
    channels: HashMap<TorServiceId, mpsc::Sender<ConnectionEvent>>,
    onion_service: OnionService,
    id: TorServiceId,
    tx: mpsc::Sender<EngineEvent>,
    rx: mpsc::Receiver<EngineEvent>,
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
        let debug = cli.debug;

        Ok(Engine {
            config: cli,
            channels: HashMap::new(),
            onion_service,
            id,
            tx,
            rx,
            debug,
        })
    }

    pub fn id(&self) -> TorServiceId {
        self.id.clone()
    }

    pub fn onion_service_address(&self) -> String {
        self.onion_service.address.clone()
    }

    pub async fn get_event(
        &mut self,
        logger: &mut StandardLogger,
    ) -> Result<Option<NetworkEvent>, anyhow::Error> {
        if let Some(engine_event) = self.rx.recv().await {
            self.handle_engine_event(engine_event, logger).await
        } else {
            Ok(None)
        }
    }

    pub async fn connection_handler(&self, stream: TcpStream, socket_addr: SocketAddr) {
        let service_id_msg = ServiceIdMessage::from(&self.onion_service.signing_key);
        let tx = self.tx.clone();
        let debug = self.debug;
        tokio::spawn(async move {
            Self::handle_accept(stream, socket_addr, service_id_msg, tx, debug).await;
        });
    }

    pub async fn send_message(
        &mut self,
        message: ChatMessage,
        logger: &mut StandardLogger,
    ) -> Result<(), anyhow::Error> {
        match self.channels.get_mut(&message.recipient.clone()) {
            Some(tx) => {
                let _ = tx.send(ConnectionEvent::Message(Box::new(message))).await;
            }
            None => {
                logger.log_error(&format!(
                    "No mpsc::Sender found for id {}",
                    message.recipient
                ));
            }
        }
        Ok(())
    }

    pub async fn connect(
        &mut self,
        address: &str,
        logger: &mut StandardLogger,
    ) -> Result<(), anyhow::Error> {
        logger.log_debug(&format!("Connecting as client to {}", address));
        // Use the proxy address for our socket address
        let socket_addr = SocketAddr::from_str(&self.config.tor_proxy_address)?;

        // Parse the address to get the ID
        let mut iter = address.rsplitn(2, ':');
        iter.next()
            .and_then(|port_str| port_str.parse::<u16>().ok())
            .ok_or(anyhow::anyhow!("Invalid port value"))?;
        let domain = iter.next().ok_or(anyhow::anyhow!("Invalid domain"))?;
        let id = TorServiceId::from_str(domain.split('.').collect::<Vec<&str>>()[0])?;

        logger.log_info(&format!("Connecting to {}...", address));

        let tx = self.tx.clone();
        let address = address.to_string();
        let debug = self.debug;
        let service_id_msg = ServiceIdMessage::from(&self.onion_service.signing_key);
        tokio::spawn(async move {
            let mut logger = TxLogger::new(&tx, debug);

            // Connect through the Tor SOCKS proxy
            let stream = match Socks5Stream::connect(socket_addr, address.clone()).await {
                Ok(stream) => stream.into_inner(),
                Err(error) => {
                    logger
                        .log_error(&format!("Error connecting to {}: {}", address, error))
                        .await;
                    return Err(error.into());
                }
            };

            // Setup the reader and writer
            let (reader, writer) = tokio::io::split(stream);

            logger
                .log_debug(&format!("Initiating client handshake to {}", address))
                .await;
            let (reader, writer, _peer_service_id) =
                match client_handshake(reader, writer, service_id_msg, &mut logger).await {
                    Ok((reader, writer, peer_service_id)) => (reader, writer, peer_service_id),
                    Err(error) => {
                        logger
                            .log_error(&format!("Error connecting to {}: {}", socket_addr, error))
                            .await;
                        return Err(error);
                    }
                };

            logger.log_info(&format!("Connected to {}", address)).await;

            // Spawn the handler
            logger.log_debug("Spawning connection handler").await;
            let connection = Connection::new(socket_addr, &id, ConnectionDirection::Outgoing);
            let (main_thread_tx, my_thread_rx) = tokio::sync::mpsc::channel(100);

            // Let the main thread know we're connected
            let _ = tx
                .send(EngineEvent::NewConnection(
                    Box::new(connection.clone()),
                    main_thread_tx,
                ))
                .await;

            Self::handle_connection(connection, reader, writer, tx, my_thread_rx, debug).await;
            Ok(())
        });

        Ok(())
    }

    pub async fn disconnect(
        &mut self,
        id: &TorServiceId,
        logger: &mut StandardLogger,
    ) -> Result<(), anyhow::Error> {
        match self.channels.get_mut(id) {
            Some(tx) => {
                tx.send(ConnectionEvent::CloseConnection).await.unwrap();
                Ok(())
            }
            None => {
                logger.log_error(&format!("Unknown connection id '{}'", id));
                Err(anyhow::anyhow!("Unknown connection id '{}'", id))
            }
        }
    }

    async fn handle_engine_event(
        &mut self,
        engine_event: EngineEvent,
        logger: &mut StandardLogger,
    ) -> Result<Option<NetworkEvent>, anyhow::Error> {
        match engine_event {
            EngineEvent::NewConnection(connection, thread_tx) => {
                self.channels
                    .insert(connection.id.clone(), thread_tx.clone());
                Ok(Some(NetworkEvent::NewConnection(connection)))
            }
            EngineEvent::Message(chat_message) => Ok(Some(NetworkEvent::Message(chat_message))),
            EngineEvent::Error(error) => {
                logger.log_error(&format!("Got network error: {}", error));
                Ok(None)
            }
            EngineEvent::ConnectionClosed(connection) => {
                match self.channels.get(&connection.id) {
                    Some(_tx) => {
                        self.channels.remove(&connection.id);
                    }
                    None => {
                        logger.log_error(&format!(
                            "Dropped unknown connection {}",
                            connection.address
                        ));
                        self.channels.remove(&connection.id);
                    }
                }
                logger.log_info(&format!("Lost connection to {}", connection.address));
                Ok(Some(NetworkEvent::ConnectionClosed(connection)))
            }
            EngineEvent::LogMessage(log_message) => {
                logger.log(log_message);
                Ok(None)
            }
        }
    }

    async fn handle_accept(
        stream: TcpStream,
        socket_addr: SocketAddr,
        service_id_msg: ServiceIdMessage,
        tx: mpsc::Sender<EngineEvent>,
        debug: bool,
    ) {
        let mut logger = TxLogger::new(&tx, debug);
        logger
            .log_debug(&format!(
                "Got connection from {}, initiating server handshake",
                socket_addr
            ))
            .await;
        let (reader, writer) = tokio::io::split(stream);

        let (reader, writer, peer_service_id) =
            match server_handshake(reader, writer, service_id_msg, &mut logger).await {
                Ok((reader, writer, peer_service_id)) => (reader, writer, peer_service_id),
                Err(error) => {
                    logger
                        .log_error(&format!(
                            "Error in incoming connection from {}: {}",
                            socket_addr, error
                        ))
                        .await;
                    return;
                }
            };

        logger
            .log_info(&format!(
                "Incoming connection from {} ({})",
                peer_service_id, socket_addr
            ))
            .await;

        let connection =
            Connection::new(socket_addr, &peer_service_id, ConnectionDirection::Incoming);
        let (main_thread_tx, my_thread_rx) = tokio::sync::mpsc::channel(100);
        logger.log_debug("Returning new connection").await;
        let _ = tx
            .send(EngineEvent::NewConnection(
                Box::new(connection.clone()),
                main_thread_tx,
            ))
            .await;
        logger.log_debug("Running connection handler").await;
        Self::handle_connection(connection, reader, writer, tx, my_thread_rx, debug).await;
    }

    async fn handle_connection(
        connection: Connection,
        mut reader: DecryptingReader<tokio::io::ReadHalf<TcpStream>>,
        mut writer: EncryptingWriter<tokio::io::WriteHalf<TcpStream>>,
        tx: mpsc::Sender<EngineEvent>,
        mut rx: mpsc::Receiver<ConnectionEvent>,
        debug: bool,
    ) {
        let mut logger = TxLogger::new(&tx, debug);
        loop {
            tokio::select! {
                result = reader.read(&mut logger) => {
                    match result {
                        Ok(Some(message)) => match message {
                            NetworkMessage::ChatMessage {
                                sender,
                                recipient,
                                message,
                            } => {
                                match TorServiceId::from_str(&sender) {
                                    Ok(sender) => {
                                        match TorServiceId::from_str(&recipient) {
                                            Ok(recipient) => {
                                                let _ = tx.send(EngineEvent::Message(Box::new(ChatMessage::new(&sender, &recipient, message)))).await;
                                            }
                                            Err(error) => {
                                                logger.log_error(&format!("Got bad service ID '{}' from message: {}", recipient, error)).await;
                                            }
                                        }
                                    }
                                    Err(error) => {
                                        logger.log_error(&format!("Got bad service ID '{}' from message: {}", sender, error)).await;
                                    }
                                }
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
                            let _ = tx.send(EngineEvent::ConnectionClosed(Box::new(connection))).await;
                            break;
                        }
                        Err(error) => {
                            let _ = tx
                                .send(EngineEvent::Error(anyhow::anyhow!(
                                    "Error reading from connection: {}",
                                    error
                                )))
                                .await;
                            let _ = tx.send(EngineEvent::ConnectionClosed(Box::new(connection))).await;
                            break;
                        }
                    }
                },
                event = rx.recv() => {
                    if let Some(event) = event {
                        match event {
                            ConnectionEvent::Message(chat_message) => {
                                if let Err(error) = writer.send(&(*chat_message).into(), &mut logger).await {
                                    logger.log_error(&format!("Error sending message: {}", error)).await;
                                }
                            },
                            ConnectionEvent::CloseConnection => {
                                logger.log_info(&format!("Disconnecting from {}", connection.id)).await;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}
