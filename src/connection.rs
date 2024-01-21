use crate::{
    crypto::{
        create_encrypted_channel, generate_auth_data, generate_session_hash, key_exchange,
        verify_auth_message, AuthMessage, DecryptingReader, EncryptingWriter,
    },
    engine::{ConnectionDirection, ConnectionEvent, ConnectionInfo, Engine, EngineEvent},
    logger::Logger,
};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr as TcpSocketAddr, str::FromStr};
use tokio::io::{AsyncRead, AsyncWrite, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_socks::tcp::Socks5Stream;
use tor_client_lib::{
    control_connection::{OnionServiceStream, SocketAddr},
    TorServiceId,
};

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ConnectionAuthorizedMessage;

pub struct Connection<T: AsyncRead + AsyncWrite> {
    connection_info: ConnectionInfo,
    reader: DecryptingReader<ReadHalf<T>>,
    writer: EncryptingWriter<WriteHalf<T>>,
    engine_tx: mpsc::UnboundedSender<EngineEvent>,
    rx: mpsc::UnboundedReceiver<ConnectionEvent>,
}

impl<T: AsyncRead + AsyncWrite> Connection<T> {
    fn new(
        connection_info: ConnectionInfo,
        reader: DecryptingReader<ReadHalf<T>>,
        writer: EncryptingWriter<WriteHalf<T>>,
        engine_tx: mpsc::UnboundedSender<EngineEvent>,
        rx: mpsc::UnboundedReceiver<ConnectionEvent>,
    ) -> Self {
        Self {
            connection_info,
            reader,
            writer,
            engine_tx,
            rx,
        }
    }

    pub async fn handle_connection(&mut self, logger: &mut dyn Logger) {
        loop {
            tokio::select! {
                result = self.reader.read() => {
                    match result {
                        Ok(Some(chat_message)) => {
                            let _ = self.engine_tx.send(EngineEvent::Message(Box::new(chat_message)));
                        },
                        Ok(None) => {
                            let _ = self.engine_tx.send(EngineEvent::ConnectionClosed(Box::new(self.connection_info.clone())));
                            break;
                        }
                        Err(error) => {
                            let _ = self.engine_tx
                                .send(EngineEvent::Error(anyhow::anyhow!(
                                    "Error reading from connection: {}",
                                    error
                                )));
                            let _ = self.engine_tx.send(EngineEvent::ConnectionClosed(Box::new(self.connection_info.clone())));
                            break;
                        }
                    }
                },
                event = self.rx.recv() => {
                    if let Some(event) = event {
                        match event {
                            ConnectionEvent::Message(chat_message) => {
                                if let Err(error) = self.writer.send(&chat_message).await {
                                    logger.log_error(&format!("Error sending message: {}", error));
                                }
                            },
                            ConnectionEvent::ConnectionAuthorized => {
                                if let Err(error) = self.writer.send(&ConnectionAuthorizedMessage).await {
                                    logger.log_error(&format!("Error sending message: {}", error));
                                }
                            },
                            ConnectionEvent::CloseConnection => {
                                logger.log_info(&format!("Disconnecting from {}", self.connection_info.id()));
                                break;
                            }
                            _ => {
                                logger.log_error(&format!("Unexpected event received: {:?}", event));
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

pub async fn connect(
    address: &str,
    proxy_address: &str,
    id: &TorServiceId,
    engine_tx: mpsc::UnboundedSender<EngineEvent>,
    logger: &mut dyn Logger,
) -> Result<Connection<TcpStream>> {
    logger.log_debug(&format!("Connecting as client to {}", address));

    // Use the proxy address for our socket address
    let socket_addr = TcpSocketAddr::from_str(proxy_address)?;

    // Parse the address to get the ID
    let mut iter = address.rsplitn(2, ':');
    iter.next()
        .and_then(|port_str| port_str.parse::<u16>().ok())
        .ok_or(anyhow::anyhow!("Invalid port value"))?;
    let domain = iter.next().ok_or(anyhow::anyhow!("Invalid domain"))?;
    let peer_id = TorServiceId::from_str(domain.split('.').collect::<Vec<&str>>()[0])?;

    // Connect through the Tor SOCKS proxy
    logger.log_info(&format!("Connecting to {}...", address));
    let stream = match Socks5Stream::connect(socket_addr, address).await {
        Ok(stream) => stream.into_inner(),
        Err(error) => {
            return Err(anyhow!("Error connecting to {}: {}", address, error));
        }
    };

    logger.log_info(&format!("Connected to {}", address));

    // Setup the reader and writer
    let (mut reader, mut writer) = tokio::io::split(stream);

    let (encryption_key, shared_secret) =
        key_exchange(&mut reader, &mut writer, true, logger).await?;

    let (mut reader, mut writer) = create_encrypted_channel(&encryption_key, reader, writer);

    let session_hash = match generate_session_hash(id, &peer_id, &shared_secret) {
        Ok(hash) => hash,
        Err(error) => {
            return Err(anyhow!("Error generating session hash: {}", error));
        }
    };

    let (main_thread_tx, rx) = mpsc::unbounded_channel();

    let auth_data = generate_auth_data(id, &session_hash);
    let signature = Engine::sign_data(&auth_data, &engine_tx).await?;
    let auth_message = AuthMessage::new(id, &signature);
    writer.send(&auth_message).await?;
    let peer_auth_message = match reader.read::<AuthMessage>().await? {
        Some(auth_message) => auth_message,
        None => {
            return Err(anyhow!("Peer disconnected during handshake"));
        }
    };
    verify_auth_message(&peer_auth_message, &peer_id, &session_hash)?;

    logger.log_debug("Waiting for connection authorized message");
    reader.read::<ConnectionAuthorizedMessage>().await?;
    logger.log_debug("Got connection authorized message");

    let connection_info =
        ConnectionInfo::new(socket_addr.into(), &peer_id, ConnectionDirection::Outgoing);

    // Let the main thread know we're connected
    engine_tx
        .send(EngineEvent::NewConnection(
            Box::new(connection_info.clone()),
            main_thread_tx,
        ))
        .unwrap();

    Ok(Connection::new(
        connection_info,
        reader,
        writer,
        engine_tx,
        rx,
    ))
}

pub async fn handle_incoming_connection(
    id: &TorServiceId,
    stream: OnionServiceStream,
    socket_addr: SocketAddr,
    engine_tx: mpsc::UnboundedSender<EngineEvent>,
    logger: &mut dyn Logger,
) -> Result<Connection<OnionServiceStream>> {
    let (mut reader, mut writer) = tokio::io::split(stream);

    let (encryption_key, shared_secret) =
        key_exchange(&mut reader, &mut writer, false, logger).await?;

    let (mut reader, mut writer) = create_encrypted_channel(&encryption_key, reader, writer);

    let (main_thread_tx, rx) = mpsc::unbounded_channel();
    let peer_auth_message = match reader.read::<AuthMessage>().await? {
        Some(auth_message) => auth_message,
        None => {
            return Err(anyhow!("Peer disconnected during handshake"));
        }
    };
    let peer_id = match TorServiceId::from_str(&peer_auth_message.service_id()) {
        Ok(service_id) => service_id,
        Err(error) => {
            return Err(anyhow!(
                "Error parsing service ID from auth message: {}",
                error
            ));
        }
    };
    let session_hash = match generate_session_hash(&peer_id, id, &shared_secret) {
        Ok(hash) => hash,
        Err(error) => {
            return Err(anyhow!("Error generating session hash: {}", error));
        }
    };
    verify_auth_message(&peer_auth_message, &peer_id, &session_hash)?;
    let auth_data = generate_auth_data(id, &session_hash);
    let signature = Engine::sign_data(&auth_data, &engine_tx).await?;
    let auth_message = AuthMessage::new(id, &signature);
    writer.send(&auth_message).await?;

    let connection_info = ConnectionInfo::new(socket_addr, &peer_id, ConnectionDirection::Incoming);

    // Let the main thread know we're connected
    engine_tx
        .send(EngineEvent::NewConnection(
            Box::new(connection_info.clone()),
            main_thread_tx,
        ))
        .unwrap();

    Ok(Connection::new(
        connection_info,
        reader,
        writer,
        engine_tx,
        rx,
    ))
}
