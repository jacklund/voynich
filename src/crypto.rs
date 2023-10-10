use chacha20poly1305;
use ed25519_dalek::{pkcs8::spki::der::zeroize::Zeroize, Signature, Signer};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tor_client_lib::key::TorEd25519SigningKey;
use x25519_dalek::{EphemeralSecret, PublicKey, SharedSecret};

#[derive(Debug, Deserialize, Serialize)]
pub enum AlgorithmIdentifier {
    X25519,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HandShake {
    algorithm_identifier: AlgorithmIdentifier,
    // For some dumb reason, PublicKey isn't serializable
    public_key: [u8; 32],
    signature: Signature,
}

impl HandShake {
    pub fn new(signing_key: &TorEd25519SigningKey, ephemeral_public_key: &PublicKey) -> HandShake {
        let signature = signing_key.sign(ephemeral_public_key.as_bytes());

        Self {
            algorithm_identifier: AlgorithmIdentifier::X25519,
            public_key: *ephemeral_public_key.as_bytes(),
            signature,
        }
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey::from(self.public_key)
    }
}

pub fn generate_ephemeral_keypair() -> (EphemeralSecret, PublicKey) {
    let secret = EphemeralSecret::random();
    let public = PublicKey::from(&secret);

    (secret, public)
}

pub fn generate_shared_secret(
    secret_key: EphemeralSecret,
    public_key: &mut PublicKey,
) -> SharedSecret {
    let shared = secret_key.diffie_hellman(public_key);
    public_key.zeroize();
    shared
}

pub fn generate_symmetric_key(
    shared: SharedSecret,
) -> Result<chacha20poly1305::Key, anyhow::Error> {
    let hkdf = Hkdf::<Sha256>::new(None, shared.as_bytes());
    let mut output = [0u8; 32];
    if let Err(hkdf::InvalidLength) = hkdf.expand("encryption".as_bytes(), &mut output) {
        return Err(anyhow::anyhow!("Invalid length"));
    }

    Ok(output.into())
}
