use serde::{Deserialize, Serialize};
use tor_client_lib::{
    control_connection::{OnionAddress, OnionServiceMapping, TorSocketAddr},
    error::TorError,
    OnionService as TorClientOnionService, TorEd25519SigningKey, TorServiceId,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum OnionType {
    Transient,
    Persistent { name: String, create: bool },
}

impl OnionType {
    pub fn new_transient() -> Self {
        Self::Transient
    }

    pub fn new_persistent(name: &str) -> Self {
        Self::Persistent {
            name: name.to_string(),
            create: true,
        }
    }

    pub fn existing_persistent(name: &str) -> Self {
        Self::Persistent {
            name: name.to_string(),
            create: false,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub struct OnionService {
    name: String,
    service: TorClientOnionService,
}

impl OnionService {
    pub fn new(name: &str, service: TorClientOnionService) -> Self {
        Self {
            name: name.to_string(),
            service,
        }
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn ports(&self) -> &Vec<OnionServiceMapping> {
        self.service.ports()
    }

    pub fn service_id(&self) -> &TorServiceId {
        self.service.service_id()
    }

    pub fn signing_key(&self) -> &TorEd25519SigningKey {
        self.service.signing_key()
    }

    pub fn listen_addresses_for_port(&self, service_port: u16) -> Vec<TorSocketAddr> {
        self.service.listen_addresses_for_port(service_port)
    }

    pub fn onion_address(&self, service_port: u16) -> Result<OnionAddress, TorError> {
        self.service.onion_address(service_port)
    }
}

impl From<TorClientOnionService> for OnionService {
    fn from(service: TorClientOnionService) -> Self {
        Self {
            name: service.service_id().to_string(),
            service,
        }
    }
}
