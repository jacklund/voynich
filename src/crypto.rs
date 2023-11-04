use crate::{chat::ChatMessage, logger::Logger};
use chacha20poly1305::{
    aead::{Aead, OsRng},
    AeadCore, ChaCha20Poly1305, Key as SymmetricKey, KeyInit, Nonce,
};
use ed25519_dalek::{pkcs8::spki::der::zeroize::Zeroize, Signature, Signer, Verifier};
use futures::{SinkExt, TryStreamExt};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::marker::Unpin;
use std::str::FromStr;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tor_client_lib::key::{TorEd25519SigningKey, TorServiceId};
use x25519_dalek::{EphemeralSecret, PublicKey, SharedSecret};

#[derive(Clone)]
pub struct Cryptor {
    cipher: ChaCha20Poly1305,
}

const NONCE_SIZE: usize = 12;

impl Cryptor {
    pub fn new(key: &SymmetricKey) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(key),
        }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        match self.cipher.encrypt(&nonce, plaintext) {
            Ok(ciphertext) => {
                let mut ret = Vec::new();
                ret.extend_from_slice(nonce.as_slice());
                ret.extend_from_slice(ciphertext.as_slice());
                Ok(ret)
            }
            Err(_) => Err(anyhow::anyhow!("Encryption error")),
        }
    }

    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
        match self.cipher.decrypt(
            Nonce::from_slice(&ciphertext[..NONCE_SIZE]),
            &ciphertext[NONCE_SIZE..],
        ) {
            Ok(plaintext) => Ok(plaintext),
            Err(_) => Err(anyhow::anyhow!("Decryption error")),
        }
    }
}

pub struct EncryptingWriter<W: AsyncWrite + Unpin> {
    writer: FramedWrite<W, LengthDelimitedCodec>,
    cryptor: Cryptor,
}

impl<W: AsyncWrite + Unpin> EncryptingWriter<W> {
    fn new(writer: W, cryptor: Cryptor) -> Self {
        Self {
            writer: FramedWrite::new(writer, LengthDelimitedCodec::new()),
            cryptor,
        }
    }

    pub async fn send<L: Logger + ?Sized>(
        &mut self,
        message: &NetworkMessage,
        logger: &mut L,
    ) -> Result<(), anyhow::Error> {
        let serialized = serde_cbor::to_vec(message)?;
        logger.log_debug(&format!(
            "Unencrypted message length = {}",
            serialized.len()
        ));
        let encrypted = self.cryptor.encrypt(&serialized)?;
        self.writer.send(encrypted.into()).await?;
        Ok(())
    }

    async fn send_service_id<L: Logger + ?Sized>(
        &mut self,
        signing_key: &TorEd25519SigningKey,
        logger: &mut L,
    ) -> Result<(), anyhow::Error> {
        let service_id_msg: ServiceIdMessage = signing_key.into();
        self.send(&service_id_msg.into(), logger).await
    }
}

#[derive(Serialize, Deserialize)]
pub enum NetworkMessage {
    ChatMessage {
        sender: String,
        recipient: String,
        message: String,
    },
    ServiceIdMessage {
        service_id: String,
        signature: Signature,
    },
}

impl From<ServiceIdMessage> for NetworkMessage {
    fn from(msg: ServiceIdMessage) -> Self {
        Self::ServiceIdMessage {
            service_id: msg.service_id,
            signature: msg.signature,
        }
    }
}

impl From<ChatMessage> for NetworkMessage {
    fn from(msg: ChatMessage) -> Self {
        Self::ChatMessage {
            sender: msg.sender.into(),
            recipient: msg.recipient.into(),
            message: msg.message,
        }
    }
}

pub struct DecryptingReader<R: AsyncRead + Unpin> {
    reader: FramedRead<R, LengthDelimitedCodec>,
    cryptor: Cryptor,
}

impl<R: AsyncRead + Unpin> DecryptingReader<R> {
    fn new(reader: R, cryptor: Cryptor) -> Self {
        Self {
            reader: FramedRead::new(reader, LengthDelimitedCodec::new()),
            cryptor,
        }
    }

    pub async fn read<L: Logger + ?Sized>(
        &mut self,
        logger: &mut L,
    ) -> Result<Option<NetworkMessage>, anyhow::Error> {
        let bytes_opt = self.reader.try_next().await?;
        let value: Option<NetworkMessage> = match bytes_opt {
            Some(ciphertext) => {
                logger.log_debug(&format!(
                    "DecryptingReader::read read {} bytes",
                    ciphertext.len()
                ));
                let plaintext = self.cryptor.decrypt(&ciphertext)?;
                serde_cbor::from_slice(&plaintext)?
            }
            None => None,
        };
        Ok(value)
    }

    async fn read_service_id<L: Logger + ?Sized>(
        &mut self,
        logger: &mut L,
    ) -> Result<Option<ServiceIdMessage>, anyhow::Error> {
        match timeout(Duration::from_secs(10), self.read(logger)).await {
            Ok(result) => match result? {
                Some(message) => match message {
                    NetworkMessage::ServiceIdMessage {
                        service_id,
                        signature,
                    } => {
                        let tor_service_id = TorServiceId::from_str(&service_id)?;
                        if tor_service_id
                            .verifying_key()
                            .verify(service_id.as_bytes(), &signature)
                            .is_err()
                        {
                            return Err(anyhow::anyhow!(
                                "Verification error for signature of peer service ID"
                            ));
                        }
                        Ok(Some(ServiceIdMessage::new(&service_id, signature)))
                    }
                    _ => Err(anyhow::anyhow!("Expected Service ID message")),
                },
                None => Ok(None),
            },
            Err(_) => Err(anyhow::anyhow!("Read timeout")),
        }
    }
}

// Generate ephemeral key pair
pub fn generate_ephemeral_keypair() -> (EphemeralSecret, PublicKey) {
    let secret = EphemeralSecret::random();
    let public = PublicKey::from(&secret);

    (secret, public)
}

// Generate the shared secret from our secret key and peer's public key
pub fn generate_shared_secret(
    secret_key: EphemeralSecret,
    public_key: &mut PublicKey,
) -> SharedSecret {
    let shared = secret_key.diffie_hellman(public_key);
    public_key.zeroize();
    shared
}

// Use an HKDF to generate the symmetric key from the shared secret
pub fn generate_symmetric_key(shared: SharedSecret) -> Result<SymmetricKey, anyhow::Error> {
    let hkdf = Hkdf::<Sha256>::new(None, shared.as_bytes());
    let mut output = [0u8; 32];
    if let Err(hkdf::InvalidLength) = hkdf.expand("encryption".as_bytes(), &mut output) {
        return Err(anyhow::anyhow!("Invalid length"));
    }

    Ok(output.into())
}

#[derive(Serialize, Deserialize)]
struct ServiceIdMessage {
    service_id: String,
    signature: Signature,
}

impl ServiceIdMessage {
    fn new(service_id: &str, signature: Signature) -> Self {
        Self {
            service_id: service_id.to_string(),
            signature,
        }
    }
}

impl From<&TorEd25519SigningKey> for ServiceIdMessage {
    fn from(signing_key: &TorEd25519SigningKey) -> Self {
        let service_id: TorServiceId = signing_key.verifying_key().into();
        let service_id_string = service_id.as_str().to_string();
        let signature = signing_key.sign(service_id_string.as_bytes());
        Self {
            service_id: service_id_string,
            signature,
        }
    }
}

const PROTOCOL_VERSION: u8 = 1;
const ALGORITHM_CHACHA20POLY1305: u8 = 0;
const KEY_LEN: usize = 32;

pub async fn send_ephemeral_public_key<T: AsyncWrite + Unpin>(
    public_key: &PublicKey,
    mut writer: T,
) -> Result<T, anyhow::Error> {
    let mut packet = vec![PROTOCOL_VERSION, ALGORITHM_CHACHA20POLY1305];
    packet.extend_from_slice(&public_key.to_bytes());
    writer.write_all(&packet).await?;

    Ok(writer)
}

pub async fn read_peer_public_key<T: AsyncRead + Unpin, L: Logger + ?Sized>(
    mut reader: T,
    logger: &mut L,
) -> Result<(PublicKey, T), anyhow::Error> {
    let mut buffer = Vec::new();
    let bytes_read = reader.read_buf(&mut buffer).await?;
    logger.log_debug(&format!("Read {} bytes of public key data", bytes_read));
    let public_key = match bytes_read {
        len if len > 0 => {
            if len == KEY_LEN + 2 {
                if buffer[0] != PROTOCOL_VERSION {
                    return Err(anyhow::anyhow!(
                        "Wrong protocol version found in public key: {}",
                        buffer[0]
                    ));
                }
                if buffer[1] != ALGORITHM_CHACHA20POLY1305 {
                    return Err(anyhow::anyhow!(
                        "Unrecognized algorithm identifier found: {}",
                        buffer[1]
                    ));
                }
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&buffer[2..]);
                PublicKey::from(bytes)
            } else {
                return Err(anyhow::anyhow!("Bad public key packet length: {}", len));
            }
        }
        len if len == 0 => {
            return Err(anyhow::anyhow!("End of file found on stream"));
        }
        len => {
            return Err(anyhow::anyhow!("Unexpected length value: {}", len));
        }
    };

    Ok((public_key, reader))
}

pub async fn client_handshake<R: AsyncRead + Unpin, W: AsyncWrite + Unpin, L: Logger + ?Sized>(
    reader: R,
    writer: W,
    signing_key: &TorEd25519SigningKey,
    logger: &mut L,
) -> Result<(DecryptingReader<R>, EncryptingWriter<W>, TorServiceId), anyhow::Error> {
    // Generate ephemeral keypair, send public key, read their public key, and generate shared
    // secret
    let (private_key, public_key) = generate_ephemeral_keypair();
    let writer = send_ephemeral_public_key(&public_key, writer).await?;
    let (mut peer_public_key, reader) = read_peer_public_key(reader, logger).await?;
    let shared_secret = generate_shared_secret(private_key, &mut peer_public_key);

    // Generate encryption key from shared secret
    let encryption_key = generate_symmetric_key(shared_secret)?;

    // Create the cryptor, the writer and reader
    let cryptor = Cryptor::new(&encryption_key);
    let mut writer = EncryptingWriter::new(writer, cryptor.clone());
    let mut reader = DecryptingReader::new(reader, cryptor.clone());

    // Send our service ID
    writer.send_service_id(signing_key, logger).await?;

    // Read peer service ID
    let peer_service_id = match reader.read_service_id(logger).await? {
        Some(service_id) => service_id,
        None => Err(anyhow::anyhow!("End of stream before reading service ID"))?,
    };

    Ok((
        reader,
        writer,
        TorServiceId::from_str(&peer_service_id.service_id)?,
    ))
}

pub async fn server_handshake<R: AsyncRead + Unpin, W: AsyncWrite + Unpin, L: Logger + ?Sized>(
    reader: R,
    writer: W,
    signing_key: &TorEd25519SigningKey,
    logger: &mut L,
) -> Result<(DecryptingReader<R>, EncryptingWriter<W>, TorServiceId), anyhow::Error> {
    // Generate ephemeral keypair, send public key, read their public key, and generate shared
    // secret
    let (mut peer_public_key, reader) = read_peer_public_key(reader, logger).await?;
    let (private_key, public_key) = generate_ephemeral_keypair();
    let writer = send_ephemeral_public_key(&public_key, writer).await?;
    let shared_secret = generate_shared_secret(private_key, &mut peer_public_key);

    // Generate encryption key from shared secret
    let encryption_key = generate_symmetric_key(shared_secret)?;
    //
    // Create the cryptor, the writer and reader
    let cryptor = Cryptor::new(&encryption_key);
    let mut writer = EncryptingWriter::new(writer, cryptor.clone());
    let mut reader = DecryptingReader::new(reader, cryptor.clone());

    // Read peer service ID
    let peer_service_id = match reader.read_service_id(logger).await? {
        Some(service_id) => service_id,
        None => Err(anyhow::anyhow!("End of stream before reading service ID"))?,
    };

    // Send our service ID
    writer.send_service_id(signing_key, logger).await?;

    Ok((
        reader,
        writer,
        TorServiceId::from_str(&peer_service_id.service_id)?,
    ))
}
