use crate::{
    crypto::{
        generate_ephemeral_keypair, generate_shared_secret, generate_symmetric_key, HandShake,
    },
    ui::{ChatMessage, InputEvent, Renderer, UI},
    Cli, TermInputStream,
};
use anyhow;
use futures::{SinkExt, StreamExt, TryStreamExt};
use hex;
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    str::FromStr,
    time::Duration,
};
use tokio;
use tokio::{
    io::{ReadHalf, WriteHalf},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_serde::{formats::Cbor, SymmetricallyFramed};
use tokio_socks::tcp::Socks5Stream;
use tokio_util::codec::{
    FramedRead, FramedWrite, LengthDelimitedCodec, LinesCodec, LinesCodecError,
};
use tor_client_lib::{
    auth::TorAuthentication,
    control_connection::{OnionService, TorControlConnection},
    key::{TorEd25519SigningKey, TorServiceId},
};
use x25519_dalek::{EphemeralSecret, PublicKey};

enum NetworkEvent {
    NewConnection(Connection),
    Message { sender: String, message: String },
    Error(std::io::Error),
    ConnectionClosed(Connection),
}

#[derive(Clone, PartialEq, Eq)]
pub struct Connection {
    address: SocketAddr,
    id: TorServiceId,
}

impl Connection {
    fn new(address: SocketAddr, id: &TorServiceId) -> Self {
        Self {
            address,
            id: id.clone(),
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn id(&self) -> TorServiceId {
        self.id.clone()
    }
}

async fn send_service_id(
    service_id: &str,
    writer: WriteHalf<TcpStream>,
) -> Result<WriteHalf<TcpStream>, anyhow::Error> {
    let mut serialized_channel = SymmetricallyFramed::new(
        FramedWrite::new(writer, LengthDelimitedCodec::new()),
        Cbor::<String, String>::default(),
    );
    serialized_channel.send(service_id.to_string()).await?;

    Ok(serialized_channel.into_inner().into_inner())
}

async fn send_handshake(
    signing_key: &TorEd25519SigningKey,
    public_key: &PublicKey,
    writer: WriteHalf<TcpStream>,
) -> Result<WriteHalf<TcpStream>, anyhow::Error> {
    let handshake = HandShake::new(signing_key, public_key);
    let mut serialized_channel = SymmetricallyFramed::new(
        FramedWrite::new(writer, LengthDelimitedCodec::new()),
        Cbor::<HandShake, HandShake>::default(),
    );
    serialized_channel.send(handshake).await?;

    Ok(serialized_channel.into_inner().into_inner())
}

async fn read_service_id(
    reader: ReadHalf<TcpStream>,
) -> Result<(String, ReadHalf<TcpStream>), anyhow::Error> {
    let mut deserialized = SymmetricallyFramed::new(
        FramedRead::new(reader, LengthDelimitedCodec::new()),
        Cbor::<String, String>::default(),
    );
    let id = match timeout(Duration::from_secs(10), deserialized.try_next()).await {
        Ok(result) => match result? {
            Some(id) => id,
            None => return Err(anyhow::anyhow!("Read timeout")),
        },
        Err(_) => return Err(anyhow::anyhow!("Read timeout")),
    };

    Ok((id, deserialized.into_inner().into_inner()))
}

async fn read_handshake(
    reader: ReadHalf<TcpStream>,
) -> Result<(HandShake, ReadHalf<TcpStream>), anyhow::Error> {
    let mut deserialized = SymmetricallyFramed::new(
        FramedRead::new(reader, LengthDelimitedCodec::new()),
        Cbor::<HandShake, HandShake>::default(),
    );
    let handshake = match timeout(Duration::from_secs(10), deserialized.try_next()).await {
        Ok(result) => match result? {
            Some(handshake) => handshake,
            None => return Err(anyhow::anyhow!("Read timeout")),
        },
        Err(_) => return Err(anyhow::anyhow!("Read timeout")),
    };

    Ok((handshake, deserialized.into_inner().into_inner()))
}

pub struct Engine {
    tor_control_connection: TorControlConnection,
    config: Cli,
    writers: HashMap<SocketAddr, WriteHalf<TcpStream>>,
    onion_service: OnionService,
    id: TorServiceId,
    secret_key: EphemeralSecret,
    public_key: PublicKey,
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

        // Generate a random X25519 keypair
        let secret = EphemeralSecret::random();
        let public = PublicKey::from(&secret);

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
        })
    }

    pub fn id(&self) -> &str {
        &self.id.as_str()
    }

    async fn handle_accept(
        &mut self,
        stream: TcpStream,
        socket_addr: SocketAddr,
        ui: &mut UI,
    ) -> Result<(), anyhow::Error> {
        let (reader, writer) = tokio::io::split(stream);
        let (id, reader) = read_service_id(reader).await?;

        // Send our handshake
        let (secret, public) = generate_ephemeral_keypair();
        let writer = send_handshake(&self.onion_service.signing_key, &public, writer).await?;

        // Read the handshake
        let (handshake, reader) = read_handshake(reader).await?;

        // Generate the shared encryption key
        let shared_key = generate_shared_secret(secret, &mut handshake.public_key());
        let encryption_key = generate_symmetric_key(shared_key)?;
        ui.log_info(&format!("shared key = {}", hex::encode(encryption_key)));

        let framed_reader = FramedRead::new(reader, LinesCodec::new());
        let connection = Connection::new(socket_addr, &TorServiceId::from_str(&id)?);
        self.writers.insert(socket_addr, writer);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            Self::handle_connection(connection, framed_reader, tx).await;
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
                            self.handle_accept(stream, socket_addr, ui).await?;
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
                            ui.log_info(&format!("Got new connection from {}, id {}", connection.address(), connection.id().as_str()));
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
                        Some(address) => self.connect(address, ui).await?,
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
                            sender: connection.id.as_str().to_string(),
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

        // Connect through the Tor SOCKS proxy
        let stream = Socks5Stream::connect(socket_addr, address)
            .await?
            .into_inner();

        // Setup the reader and writer
        let (reader, writer) = tokio::io::split(stream);

        // Send our TorServiceId
        ui.log_info("Sending Tor service ID");
        let writer = send_service_id(self.onion_service.service_id.as_str(), writer).await?;

        // Read the handshake
        ui.log_info("Reading handshake");
        let (handshake, reader) = read_handshake(reader).await?;

        // Generate the shared encryption key
        let (secret, public) = generate_ephemeral_keypair();
        let shared_key = generate_shared_secret(secret, &mut handshake.public_key());
        let encryption_key = generate_symmetric_key(shared_key)?;
        ui.log_info(&format!("shared key = {}", hex::encode(encryption_key)));

        // Send our handshake
        ui.log_info("Sending handshake");
        let writer = send_handshake(&self.onion_service.signing_key, &public, writer).await?;

        // Spawn the handler
        let connection = Connection::new(socket_addr, &id);
        self.writers.insert(socket_addr, writer);
        let tx = self.tx.clone();
        let framed_reader = FramedRead::new(reader, LinesCodec::new());
        tokio::spawn(async move {
            Self::handle_connection(connection, framed_reader, tx).await;
        });

        Ok(())
    }
}
