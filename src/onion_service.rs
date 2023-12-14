use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use tor_client_lib::{
    control_connection::OnionServiceMapping, OnionService as TorClientOnionService,
    TorEd25519SigningKey,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OnionService {
    name: String,
    ports: Vec<OnionServiceMapping>,
    hostname: String,
    secret_key: TorEd25519SigningKey,
    public_key: VerifyingKey,
}

impl OnionService {
    pub fn new(
        name: String,
        ports: Vec<OnionServiceMapping>,
        hostname: String,
        secret_key: TorEd25519SigningKey,
        public_key: VerifyingKey,
    ) -> Self {
        Self {
            name,
            ports,
            hostname,
            secret_key,
            public_key,
        }
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn ports(&self) -> &Vec<OnionServiceMapping> {
        &self.ports
    }

    pub fn hostname(&self) -> &String {
        &self.hostname
    }

    pub fn secret_key(&self) -> &TorEd25519SigningKey {
        &self.secret_key
    }

    pub fn public_key(&self) -> &VerifyingKey {
        &self.public_key
    }
}

impl From<TorClientOnionService> for OnionService {
    fn from(service: TorClientOnionService) -> Self {
        Self {
            name: service.service_id().to_string(),
            ports: service.ports().clone(),
            hostname: format!("{}.onion", service.service_id()),
            secret_key: service.signing_key().clone(),
            public_key: service.signing_key().verifying_key(),
        }
    }
}
