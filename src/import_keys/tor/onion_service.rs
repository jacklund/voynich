use crate::tor::torrc::find_torrc_onion_service;
use anyhow::{anyhow, Result};
use ed25519_dalek::VerifyingKey;
use std::fs::{read_to_string, File};
use std::io::Read;
use tor_client_lib::TorEd25519SigningKey;
use trithemius::onion_service::OnionService;

#[derive(Debug)]
pub struct OnionServiceData {
    pub hostname: String,
    pub secret_key: TorEd25519SigningKey,
    pub public_key: VerifyingKey,
}

fn read_private_key_file(directory: &str) -> Result<TorEd25519SigningKey> {
    let mut file = File::open(format!("{}/hs_ed25519_secret_key", directory))?;
    let mut data = Vec::<u8>::new();
    file.read_to_end(&mut data)?;
    println!("{:?}", data);
    println!("{:?}", &data[32..]);

    Ok(TorEd25519SigningKey::from_bytes(
        data[32..].try_into().unwrap(),
    ))
}

fn read_public_key_file(directory: &str) -> Result<VerifyingKey> {
    let mut file = File::open(format!("{}/hs_ed25519_public_key", directory))?;
    let mut data = Vec::<u8>::new();
    file.read_to_end(&mut data)?;
    println!("{:?}", data);
    println!("{:?}", &data[32..]);

    Ok(VerifyingKey::from_bytes(data[32..].try_into().unwrap())?)
}

pub fn parse_onion_service(directory: &str) -> Result<OnionServiceData> {
    if let Err(error) = sudo::escalate_if_needed() {
        return Err(anyhow!("Error sudoing: {}", error));
    }
    let hostname = read_to_string(format!("{}/hostname", directory))?
        .trim_end()
        .to_string();

    let secret_key = read_private_key_file(directory)?;
    let public_key = read_public_key_file(directory)?;

    Ok(OnionServiceData {
        hostname,
        secret_key,
        public_key,
    })
}

pub fn find_static_onion_service(name: &str) -> Result<Option<OnionService>> {
    let info = find_torrc_onion_service(name)?;
    match info {
        Some(info) => {
            let data = parse_onion_service(info.dir())?;
            Ok(Some(OnionService::new(
                info.name().to_string(),
                info.ports().clone(),
                data.hostname,
                data.secret_key,
                data.public_key,
            )))
        }
        None => Ok(None),
    }
}
