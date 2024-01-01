use crate::logger::Logger;
use anyhow::{anyhow, Result};
use chacha20poly1305::{
    aead::{Aead, OsRng},
    AeadCore, ChaCha20Poly1305, Key as SymmetricKey, KeyInit, Nonce,
};
use ed25519_dalek::{pkcs8::spki::der::zeroize::Zeroize, Signature, Verifier};
use futures::{SinkExt, TryStreamExt};
use hkdf::Hkdf;
use rand::Rng;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::marker::Unpin;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tor_client_lib::key::TorServiceId;
use x25519_dalek::{EphemeralSecret, PublicKey, SharedSecret};

/// ChaCha20Poly1305 block size (in bytes)
const BLOCKSIZE: usize = 64;

// We can store the padding length in one byte
type PaddingLength = u8;

/// Size of padding length header
const HEADER_SIZE: usize = std::mem::size_of::<PaddingLength>();

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

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        match self.cipher.encrypt(&nonce, plaintext) {
            Ok(ciphertext) => {
                let mut ret = Vec::new();
                ret.extend_from_slice(nonce.as_slice());
                ret.extend_from_slice(ciphertext.as_slice());
                Ok(ret)
            }
            Err(_) => Err(anyhow!("Encryption error")),
        }
    }

    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        match self.cipher.decrypt(
            Nonce::from_slice(&ciphertext[..NONCE_SIZE]),
            &ciphertext[NONCE_SIZE..],
        ) {
            Ok(plaintext) => Ok(plaintext),
            Err(_) => Err(anyhow!("Decryption error")),
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

    pub async fn send<S: Serialize>(&mut self, message: &S) -> Result<()> {
        let serialized = serde_cbor::to_vec(message)?;

        // Make the packet length an integral number of blocks > the message length + header
        let packet_length: usize = (((serialized.len() + HEADER_SIZE) as f64 / BLOCKSIZE as f64)
            .ceil()) as usize
            * BLOCKSIZE;

        // Figure out how much padding we need
        let padding_length: PaddingLength =
            (packet_length - serialized.len() - HEADER_SIZE) as PaddingLength;

        // Put together the packet
        // First comes the header
        let mut packet: Vec<u8> = padding_length.to_be_bytes().to_vec();

        // Next the data
        packet.extend_from_slice(&serialized);

        // Pad with random bytes up to the block size
        for _ in 0..padding_length {
            packet.push(OsRng.gen());
        }

        // Encrypt and send
        let encrypted = self.cryptor.encrypt(&packet)?;
        self.writer.send(encrypted.into()).await?;

        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthMessage {
    service_id: String,
    signature: Signature,
}

impl AuthMessage {
    pub fn new(service_id: &TorServiceId, signature: &Signature) -> Self {
        Self {
            service_id: service_id.to_string(),
            signature: signature.clone(),
        }
    }

    pub fn service_id(&self) -> String {
        self.service_id.clone()
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

    pub async fn read<D: DeserializeOwned>(&mut self) -> Result<Option<D>> {
        match self.reader.try_next().await? {
            Some(ciphertext) => {
                // Decrypt the packet
                let plaintext = self.cryptor.decrypt(&ciphertext)?;

                // Read the packet length header
                let padding_length =
                    PaddingLength::from_be_bytes(plaintext[..HEADER_SIZE].try_into().unwrap());

                // Figure out the message length
                let message_len = plaintext.len() - HEADER_SIZE - padding_length as usize;

                // Deserialize the message
                Ok(Some(serde_cbor::from_slice(
                    &plaintext[HEADER_SIZE..message_len + HEADER_SIZE],
                )?))
            }
            None => Ok(None),
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
pub fn generate_symmetric_key(shared: &SharedSecret) -> Result<SymmetricKey> {
    let hkdf = Hkdf::<Sha256>::new(None, shared.as_bytes());
    let mut output = [0u8; 32];
    if let Err(hkdf::InvalidLength) = hkdf.expand("encryption".as_bytes(), &mut output) {
        return Err(anyhow!("Invalid length"));
    }

    Ok(output.into())
}

const PROTOCOL_VERSION: u8 = 1;
const ALGORITHM_CHACHA20POLY1305: u8 = 0;
const KEY_LEN: usize = 32;

pub async fn send_ephemeral_public_key<T: AsyncWrite + Unpin>(
    public_key: &PublicKey,
    writer: &mut T,
) -> Result<()> {
    let mut packet = vec![PROTOCOL_VERSION, ALGORITHM_CHACHA20POLY1305];
    packet.extend_from_slice(&public_key.to_bytes());
    writer.write_all(&packet).await?;

    Ok(())
}

pub async fn read_peer_public_key<T: AsyncRead + Unpin>(
    reader: &mut T,
    logger: &mut dyn Logger,
) -> Result<PublicKey> {
    let mut buffer = Vec::new();
    let bytes_read = match timeout(Duration::from_secs(10), reader.read_buf(&mut buffer)).await {
        Ok(Ok(bytes_read)) => bytes_read,
        Ok(Err(error)) => Err(error)?,
        Err(_) => Err(anyhow!("Read timeout"))?,
    };
    logger.log_debug(&format!("Read {} bytes of public key data", bytes_read));
    let public_key = match bytes_read {
        len if len > 0 => {
            if len == KEY_LEN + 2 {
                if buffer[0] != PROTOCOL_VERSION {
                    return Err(anyhow!(
                        "Wrong protocol version found in public key: {}",
                        buffer[0]
                    ));
                }
                if buffer[1] != ALGORITHM_CHACHA20POLY1305 {
                    return Err(anyhow!(
                        "Unrecognized algorithm identifier found: {}",
                        buffer[1]
                    ));
                }
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&buffer[2..]);
                PublicKey::from(bytes)
            } else {
                return Err(anyhow!("Bad public key packet length: {}", len));
            }
        }
        0 => {
            return Err(anyhow!("End of file found on stream"));
        }
        len => {
            return Err(anyhow!("Unexpected length value: {}", len));
        }
    };

    Ok(public_key)
}

pub type SessionHash = Vec<u8>;

pub fn generate_session_hash(
    client_id: &TorServiceId,
    server_id: &TorServiceId,
    shared_secret: &SharedSecret,
) -> Result<SessionHash> {
    let client_public_key = match client_id.verifying_key() {
        Ok(key) => key,
        Err(error) => {
            return Err(anyhow!(
                "Error getting public key from service ID {}: {}",
                client_id,
                error
            ));
        }
    };
    let server_public_key = match server_id.verifying_key() {
        Ok(key) => key,
        Err(error) => {
            return Err(anyhow!(
                "Error getting public key from service ID {}: {}",
                server_id,
                error
            ));
        }
    };

    let mut hasher = Sha256::new();
    hasher.update(client_id.to_string().as_bytes());
    hasher.update(server_id.to_string().as_bytes());
    hasher.update(client_public_key.as_bytes());
    hasher.update(server_public_key.as_bytes());
    hasher.update(shared_secret.as_bytes());

    Ok(hasher.finalize().to_vec())
}

pub async fn key_exchange<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut reader: &mut R,
    mut writer: &mut W,
    as_client: bool,
    logger: &mut dyn Logger,
) -> Result<(SymmetricKey, SharedSecret)> {
    let (private_key, public_key) = generate_ephemeral_keypair();
    let mut peer_public_key = if as_client {
        send_ephemeral_public_key(&public_key, &mut writer).await?;
        read_peer_public_key(&mut reader, logger).await?
    } else {
        let peer_public_key = read_peer_public_key(&mut reader, logger).await?;
        send_ephemeral_public_key(&public_key, &mut writer).await?;
        peer_public_key
    };
    let shared_secret = generate_shared_secret(private_key, &mut peer_public_key);

    // Generate encryption key from shared secret
    let encryption_key = generate_symmetric_key(&shared_secret)?;

    Ok((encryption_key, shared_secret))
}

pub fn generate_auth_data(id: &TorServiceId, session_hash: &SessionHash) -> Vec<u8> {
    let mut auth_data = Vec::new();
    auth_data.extend_from_slice(&session_hash);
    auth_data.extend_from_slice(id.to_string().as_bytes());

    auth_data
}

pub fn verify_auth_message(
    auth_message: &AuthMessage,
    peer_id: &TorServiceId,
    session_hash: &SessionHash,
) -> Result<()> {
    let verifying_key = peer_id.verifying_key()?;
    let auth_data = generate_auth_data(&peer_id, session_hash);
    if peer_id.to_string() == auth_message.service_id {
        verifying_key.verify(&auth_data, &auth_message.signature)?;
        Ok(())
    } else {
        Err(anyhow!("ID sent in auth message doesn't match peer's ID"))
    }
}

pub fn create_encrypted_channel<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    encryption_key: &SymmetricKey,
    reader: R,
    writer: W,
) -> (DecryptingReader<R>, EncryptingWriter<W>) {
    // Create the cryptor, the writer and reader
    let cryptor = Cryptor::new(&encryption_key);
    let writer = EncryptingWriter::new(writer, cryptor.clone());
    let reader = DecryptingReader::new(reader, cryptor.clone());

    (reader, writer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ChatMessage;
    use anyhow::Result;
    use chacha20poly1305::{aead::OsRng, ChaCha20Poly1305};
    use ed25519_dalek::{Signer, SigningKey};
    use std::io::Cursor;

    async fn generate_and_test_message(msg: &str) -> Result<()> {
        let mut buf = Vec::<u8>::new();
        let cursor = Cursor::new(&mut buf);
        let message = ChatMessage::new(
            &TorServiceId::generate(),
            &TorServiceId::generate(),
            msg.to_string(),
        );
        let key = ChaCha20Poly1305::generate_key(&mut OsRng);
        let cryptor = Cryptor::new(&key);
        let mut writer = EncryptingWriter::new(cursor, cryptor.clone());
        writer.send(&message).await?;
        let cursor = Cursor::new(&mut buf);
        let mut reader = DecryptingReader::new(cursor, cryptor.clone());
        let read_message = reader.read().await?;
        assert_eq!(message, read_message.unwrap());

        Ok(())
    }

    // Test encryption, decryption, and padding
    #[tokio::test]
    async fn test_read_write_encrypted() -> Result<()> {
        // message size < blocksize
        generate_and_test_message("The quick brown fox jumped over the").await?;

        // message size == blocksize
        generate_and_test_message("The quick brown fox jumped over the l").await?;

        // message size > blocksize
        generate_and_test_message("The quick brown fox jumped over the lazy dog").await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_auth() -> Result<()> {
        // Generate server key and ID
        let server_signing_key = SigningKey::generate(&mut OsRng);
        let server_id: TorServiceId = server_signing_key.verifying_key().into();

        // Generate client key and ID
        let client_signing_key = SigningKey::generate(&mut OsRng);
        let client_id: TorServiceId = client_signing_key.verifying_key().into();

        // Generate keypairs, shared secrets, and session_hashes
        let (client_private_key, mut client_public_key) = generate_ephemeral_keypair();
        let (server_private_key, mut server_public_key) = generate_ephemeral_keypair();
        let client_shared_secret =
            generate_shared_secret(client_private_key, &mut server_public_key);
        let server_shared_secret =
            generate_shared_secret(server_private_key, &mut client_public_key);
        let client_session_hash =
            generate_session_hash(&client_id, &server_id, &client_shared_secret)?;
        let server_session_hash =
            generate_session_hash(&client_id, &server_id, &server_shared_secret)?;

        // Shared secrets better be the same!
        assert_eq!(
            server_shared_secret.as_bytes(),
            client_shared_secret.as_bytes()
        );

        // Generate client auth message
        let client_auth_data = generate_auth_data(&client_id, &client_session_hash);
        let client_signature = client_signing_key.sign(&client_auth_data);
        let client_auth_message = AuthMessage::new(&client_id, &client_signature);

        // Generate server auth message
        let server_auth_data = generate_auth_data(&server_id, &server_session_hash);
        let server_signature = server_signing_key.sign(&server_auth_data);
        let server_auth_message = AuthMessage::new(&server_id, &server_signature);

        // Verify both
        verify_auth_message(&client_auth_message, &client_id, &client_session_hash)?;
        verify_auth_message(&server_auth_message, &server_id, &server_session_hash)?;

        Ok(())
    }
}
