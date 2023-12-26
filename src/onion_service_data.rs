use crate::torrc::find_torrc_onion_service;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs::{read_to_string, File};
use std::io::Read;
use tor_client_lib::TorEd25519SigningKey;

#[derive(Debug, Deserialize, Serialize)]
pub struct OnionServiceData {
    pub name: String,
    pub hostname: String,
    pub secret_key: TorEd25519SigningKey,
}

fn read_private_key_file(directory: &str) -> Result<TorEd25519SigningKey> {
    let mut file = File::open(format!("{}/hs_ed25519_secret_key", directory))?;
    let mut data = Vec::<u8>::new();
    file.read_to_end(&mut data)?;

    Ok(TorEd25519SigningKey::from_bytes(
        data[32..].try_into().unwrap(),
    ))
}

pub fn parse_onion_service(name: &str, directory: &str) -> Result<OnionServiceData> {
    let hostname = read_to_string(format!("{}/hostname", directory))?
        .trim_end()
        .to_string();

    let secret_key = read_private_key_file(directory)?;

    Ok(OnionServiceData {
        name: name.to_string(),
        hostname,
        secret_key,
    })
}

pub fn find_static_onion_service(name: &str) -> Result<Option<OnionServiceData>> {
    let info = find_torrc_onion_service(name)?;
    match info {
        Some(info) => Ok(Some(parse_onion_service(name, info.dir())?)),
        None => Ok(None),
    }
}
