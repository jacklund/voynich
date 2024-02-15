use crate::onion_service::OnionService;
use anyhow::{anyhow, Result};
use clap::crate_name;
use lazy_static::lazy_static;
use std::env;
use std::fs::{create_dir, read, read_to_string, set_permissions, write, Permissions};
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tokio_socks::tcp::Socks5Stream;
use tor_client_lib::{
    control_connection::{
        OnionAddress, OnionService as TorClientOnionService, OnionServiceListener,
        OnionServiceMapping, SocketAddr as OnionSocketAddr,
    },
    TorEd25519SigningKey,
};

lazy_static! {
    pub static ref HOME: String = match env::var("HOME") {
        Ok(value) => value,
        Err(error) => {
            panic!("Error finding home directory: {}", error);
        }
    };
    pub static ref CONFIG_HOME: String = match env::var("XDG_CONFIG_HOME") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => format!("{}/.config", *HOME),
        Err(error) => {
            panic!("Error getting value of XDG_DATA_HOME: {}", error);
        }
    };
    pub static ref DATA_DIR: String = format!("{}/.{}", *HOME, crate_name!());
}

fn create_secure_dir(path_string: &str) -> Result<()> {
    let path = Path::new(path_string);
    if path.exists() {
        if !path.is_dir() {
            return Err(anyhow!("{} is not a directory", *DATA_DIR));
        }
    } else {
        create_dir(path_string)?;
        set_permissions(path_string, Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn check_directory(dir: &str) -> Result<()> {
    let dir_path = Path::new(dir);
    if !dir_path.exists() {
        return Err(anyhow!("Directory {} doesn't exist", dir));
    }
    if !dir_path.is_dir() {
        return Err(anyhow!("{} is not a directory", dir));
    }
    let metadata = dir_path.metadata()?;
    let mode = metadata.permissions().mode();
    if mode & 0o777 != 0o700 {
        return Err(anyhow!("Permissions on {} are too permissive!", *DATA_DIR));
    }

    Ok(())
}

fn check_file(filename: &str) -> Result<PathBuf> {
    let path = Path::new(filename);
    if !path.exists() {
        return Err(anyhow!("{} doesn't exist", filename));
    }
    let metadata = path.metadata()?;
    let mode = metadata.permissions().mode();
    if mode & 0o777 != 0o600 {
        return Err(anyhow!("Permissions on {} are too permissive!", filename));
    }

    Ok(path.to_path_buf())
}

fn read_secret_key_file(dir: &str) -> Result<TorEd25519SigningKey> {
    let filename = format!("{}/ed25519_secret_key", dir);
    let path = check_file(&filename)?;
    match read(path) {
        Ok(data) => {
            let data: [u8; 64] = match data.try_into() {
                Ok(data) => data,
                Err(_) => {
                    return Err(anyhow!(
                        "Error reading {}: Data not an Ed25519 key",
                        filename
                    ));
                }
            };
            Ok(TorEd25519SigningKey::from_bytes(data))
        }
        Err(error) => Err(anyhow!("{}", error)),
    }
}

fn write_secret_key_file(dir: &str, key: &TorEd25519SigningKey) -> Result<()> {
    let filename = format!("{}/ed25519_secret_key", dir);
    let path = Path::new(&filename);
    write(path, key.to_bytes())?;
    set_permissions(path, Permissions::from_mode(0o600))?;
    Ok(())
}

fn read_onion_address_file(dir: &str) -> Result<OnionAddress> {
    let filename = format!("{}/onion_address", dir);
    let path = check_file(&filename)?;
    match read_to_string(path) {
        Ok(data) => Ok(OnionAddress::from_str(&data)?),
        Err(error) => Err(anyhow!("{}", error)),
    }
}

fn write_onion_address_file(dir: &str, address: &OnionAddress) -> Result<()> {
    let filename = format!("{}/onion_address", dir);
    let path = Path::new(&filename);
    write(path, address.to_string())?;
    set_permissions(path, Permissions::from_mode(0o600))?;
    Ok(())
}

pub fn get_onion_address(name: &str) -> Result<OnionAddress> {
    let onion_service_dir = get_onion_service_dir(name)?;
    read_onion_address_file(&onion_service_dir)
}

pub fn save_onion_address(name: &str, address: &OnionAddress) -> Result<()> {
    let dir_name = create_onion_service_dir(name)?;
    write_onion_address_file(&dir_name, address)
}

pub fn get_onion_service_key(name: &str) -> Result<TorEd25519SigningKey> {
    let onion_service_dir = get_onion_service_dir(name)?;
    read_secret_key_file(&onion_service_dir)
}

pub fn save_onion_service_key(name: &str, key: &TorEd25519SigningKey) -> Result<()> {
    let dir_name = create_onion_service_dir(name)?;
    write_secret_key_file(&dir_name, key)
}

fn get_onion_service_dir(name: &str) -> Result<String> {
    check_directory(&DATA_DIR)?;
    let onion_service_dir = format!("{}/{}", *DATA_DIR, name);
    check_directory(&onion_service_dir)?;
    Ok(onion_service_dir)
}

fn create_onion_service_dir(name: &str) -> Result<String> {
    create_secure_dir(&DATA_DIR)?;
    let path_string = format!("{}/{}", *DATA_DIR, name);
    create_secure_dir(&path_string)?;
    Ok(path_string)
}

pub fn get_onion_service(
    name: &str,
    onion_address: &OnionAddress,
    listen_address: &OnionSocketAddr,
) -> Result<OnionService, anyhow::Error> {
    let onion_service_key = get_onion_service_key(name)?;
    let onion_service_mapping =
        OnionServiceMapping::new(onion_address.service_port(), Some(listen_address.clone()));
    Ok(OnionService::new(
        name,
        TorClientOnionService::new(
            onion_address.service_id().clone(),
            onion_service_key,
            &[onion_service_mapping],
        ),
    ))
}

pub fn save_onion_service(onion_service: &OnionService, service_port: u16) -> Result<()> {
    save_onion_address(
        onion_service.name(),
        &onion_service.onion_address(service_port)?,
    )?;
    save_onion_service_key(onion_service.name(), onion_service.signing_key())
}

pub async fn test_onion_service_connection(
    listener: OnionServiceListener,
    tor_proxy_address: &SocketAddr,
    onion_address: &OnionAddress,
) -> Result<OnionServiceListener, anyhow::Error> {
    println!(
        "Testing onion service connection to {}. Please be patient, this may take a few moments...",
        onion_address
    );
    let handle = tokio::spawn(async move {
        match listener.accept().await {
            Ok(_) => Ok(listener),
            Err(error) => Err(error),
        }
    });
    Socks5Stream::connect(tor_proxy_address, onion_address.to_string()).await?;

    Ok(handle.await??)
}
