use crate::{
    chat::ChatMessage,
    connection::{connect, handle_incoming_connection},
    logger::{Level, LogMessage, Logger},
    onion_service::OnionService,
};
use anyhow::{anyhow, Result};
use ed25519_dalek::{Signature, Signer};
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tor_client_lib::{
    control_connection::{OnionAddress, OnionServiceStream, SocketAddr as TorSocketAddr},
    TorServiceId,
};

pub enum EngineEvent {
    NewConnection(Box<ConnectionInfo>, mpsc::UnboundedSender<ConnectionEvent>),
    SignatureRequest {
        tx: mpsc::UnboundedSender<ConnectionEvent>,
        data_to_be_signed: Vec<u8>,
    },
    Message(Box<ChatMessage>),
    Error(anyhow::Error),
    ConnectionClosed(Box<ConnectionInfo>),
    LogMessage(LogMessage),
}

#[derive(Debug)]
pub enum ConnectionEvent {
    Message(Box<ChatMessage>),
    SignatureResponse(Signature),
    ConnectionAuthorized,
    CloseConnection,
}

pub enum NetworkEvent {
    NewConnection(Box<ConnectionInfo>),
    Message(Box<ChatMessage>),
    ConnectionClosed(Box<ConnectionInfo>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionDirection {
    Incoming,
    Outgoing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionInfo {
    address: TorSocketAddr,
    id: TorServiceId,
    direction: ConnectionDirection,
}

impl ConnectionInfo {
    pub fn new(address: TorSocketAddr, id: &TorServiceId, direction: ConnectionDirection) -> Self {
        Self {
            address,
            id: id.clone(),
            direction,
        }
    }

    pub fn id(&self) -> TorServiceId {
        self.id.clone()
    }

    pub fn direction(&self) -> &ConnectionDirection {
        &self.direction
    }
}

pub struct TxLogger {
    tx: mpsc::UnboundedSender<EngineEvent>,
    log_level: Level,
}

impl Logger for TxLogger {
    fn log(&mut self, message: LogMessage) {
        if message.level >= self.log_level {
            self.tx.send(EngineEvent::LogMessage(message)).unwrap();
        }
    }

    fn set_log_level(&mut self, level: Level) {
        self.log_level = level;
    }
}

impl TxLogger {
    fn new(tx: &mpsc::UnboundedSender<EngineEvent>, debug: bool) -> Self {
        Self {
            tx: tx.clone(),
            log_level: if debug { Level::Debug } else { Level::Info },
        }
    }
}

pub struct Engine {
    channels: HashMap<TorServiceId, mpsc::UnboundedSender<ConnectionEvent>>,
    onion_service: OnionService,
    onion_service_address: OnionAddress,
    tor_proxy_address: SocketAddr,
    id: TorServiceId,
    tx: mpsc::UnboundedSender<EngineEvent>,
    rx: mpsc::UnboundedReceiver<EngineEvent>,
    debug: bool,
}

impl Engine {
    pub async fn new(
        onion_service: &mut OnionService,
        onion_service_address: OnionAddress,
        tor_proxy_address: SocketAddr,
        debug: bool,
    ) -> Result<Self> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let id = onion_service.service_id().clone();

        Ok(Engine {
            channels: HashMap::new(),
            onion_service: onion_service.clone(),
            onion_service_address,
            tor_proxy_address,
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
        self.onion_service_address.to_string()
    }

    pub async fn get_event(&mut self, logger: &mut dyn Logger) -> Result<Option<NetworkEvent>> {
        if let Some(engine_event) = self.rx.recv().await {
            self.handle_engine_event(engine_event, logger).await
        } else {
            Ok(None)
        }
    }

    pub async fn handle_incoming_connection(
        &self,
        stream: OnionServiceStream,
        socket_addr: TorSocketAddr,
    ) {
        let tx = self.tx.clone();
        let debug = self.debug;
        let id = self.id.clone();
        tokio::spawn(async move {
            let mut logger = TxLogger::new(&tx, debug);
            let mut connection =
                match handle_incoming_connection(&id, stream, socket_addr, tx, &mut logger).await {
                    Ok(connection) => connection,
                    Err(error) => {
                        logger.log_error(&format!("Error handling incoming connection: {}", error));
                        return;
                    }
                };

            connection.handle_connection(&mut logger).await;
        });
    }

    pub async fn send_message(
        &mut self,
        message: ChatMessage,
        logger: &mut dyn Logger,
    ) -> Result<()> {
        match self.channels.get_mut(&message.recipient.clone()) {
            Some(tx) => {
                let _ = tx.send(ConnectionEvent::Message(Box::new(message)));
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

    pub async fn sign_data(
        data_to_be_signed: &[u8],
        engine_tx: &mpsc::UnboundedSender<EngineEvent>,
    ) -> Result<Signature> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let event = EngineEvent::SignatureRequest {
            tx,
            data_to_be_signed: data_to_be_signed.to_vec(),
        };
        engine_tx.send(event).unwrap();
        match rx.recv().await {
            Some(ConnectionEvent::SignatureResponse(signature)) => Ok(signature),
            Some(event) => Err(anyhow!(
                "Got unexpected event {:?} in response to signature request",
                event
            )),
            None => Err(anyhow!(
                "Engine closed connection before servicing signature request"
            )),
        }
    }

    pub async fn connect(&mut self, address: &str) -> Result<()> {
        let tx = self.tx.clone();
        let address = address.to_string();
        let debug = self.debug;
        let proxy_address = self.tor_proxy_address;
        let id = self.id.clone();
        tokio::spawn(async move {
            let mut logger = TxLogger::new(&tx, debug);

            let mut connection = match connect(&address, &proxy_address, &id, tx, &mut logger).await
            {
                Ok(connection) => connection,
                Err(error) => {
                    logger.log_error(&format!("Error connecting to {}: {}", address, error));
                    return;
                }
            };

            connection.handle_connection(&mut logger).await;
        });

        Ok(())
    }

    pub async fn send_connection_authorized_message(
        &mut self,
        id: &TorServiceId,
        logger: &mut dyn Logger,
    ) -> Result<()> {
        match self.channels.get_mut(id) {
            Some(tx) => {
                tx.send(ConnectionEvent::ConnectionAuthorized).unwrap();
                Ok(())
            }
            None => {
                logger.log_error(&format!("Unknown connection id '{}'", id));
                Err(anyhow::anyhow!("Unknown connection id '{}'", id))
            }
        }
    }

    pub async fn disconnect(&mut self, id: &TorServiceId, logger: &mut dyn Logger) -> Result<()> {
        match self.channels.get_mut(id) {
            Some(tx) => {
                tx.send(ConnectionEvent::CloseConnection).unwrap();
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
        logger: &mut dyn Logger,
    ) -> Result<Option<NetworkEvent>> {
        match engine_event {
            EngineEvent::NewConnection(connection, thread_tx) => {
                logger.log_debug(&format!("Got new connection from {}", connection.id()));
                self.channels
                    .insert(connection.id.clone(), thread_tx.clone());
                Ok(Some(NetworkEvent::NewConnection(connection)))
            }
            EngineEvent::SignatureRequest {
                tx,
                data_to_be_signed,
            } => {
                let signature = self.onion_service.signing_key().sign(&data_to_be_signed);
                tx.send(ConnectionEvent::SignatureResponse(signature))
                    .unwrap();
                Ok(None)
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
}
