use anyhow::Result;
use ed25519_dalek::VerifyingKey;
use std::fs::{read_to_string, File};
use std::io::Read;
use tor_client_lib::TorEd25519SigningKey;
use trithemius::torrc::find_torrc_onion_service;

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

    Ok(TorEd25519SigningKey::from_bytes(
        data[32..].try_into().unwrap(),
    ))
}

fn read_public_key_file(directory: &str) -> Result<VerifyingKey> {
    let mut file = File::open(format!("{}/hs_ed25519_public_key", directory))?;
    let mut data = Vec::<u8>::new();
    file.read_to_end(&mut data)?;

    Ok(VerifyingKey::from_bytes(data[32..].try_into().unwrap())?)
}

pub fn parse_onion_service(directory: &str) -> Result<OnionServiceData> {
    let hostname = read_to_string(format!("{}/hostname", directory))?
        .trim_end()
        .to_string();

    let secret_key = read_private_key_file(directory)?;
    let public_key = read_public_key_file(directory)?;
    // println!("Setting uid, euid to {}, {}", uid, euid);

    Ok(OnionServiceData {
        hostname,
        secret_key,
        public_key,
    })
}

pub fn find_static_onion_service(name: &str) -> Result<Option<OnionServiceData>> {
    let info = find_torrc_onion_service(name)?;
    match info {
        Some(info) => Ok(Some(parse_onion_service(info.dir())?)),
        None => Ok(None),
    }
}
