use anyhow::Result;
use clap::crate_name;
use lazy_static::lazy_static;
use std::env;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::{net::SocketAddr, str::FromStr};
use tokio::net::TcpListener;
use tokio_socks::tcp::Socks5Stream;
use tor_client_lib::control_connection::TorControlConnection;
use tor_client_lib::OnionService;

lazy_static! {
    pub static ref HOME: String = match env::var("HOME") {
        Ok(value) => value,
        Err(error) => {
            panic!("Error finding home directory: {}", error);
        }
    };
    pub static ref DATA_HOME: String = match env::var("XDG_DATA_HOME") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => format!("{}/.local/share", *HOME),
        Err(error) => {
            panic!("Error getting value of XDG_DATA_HOME: {}", error);
        }
    };
}

pub fn data_dir() -> String {
    format!("{}/{}", *DATA_HOME, crate_name!())
}

pub struct OnionServicesFile {}

impl OnionServicesFile {
    fn filename() -> String {
        format!("{}/onion_services", data_dir())
    }

    pub fn exists() -> bool {
        Path::new(&Self::filename()).exists()
    }

    pub fn read() -> Result<Vec<OnionService>> {
        if Self::exists() {
            let file = File::open(Self::filename())?;
            if file.metadata()?.permissions().mode() != 0600 {
                Err(anyhow::anyhow!(
                    "Error, permissions on file {} are too permissive",
                    Self::filename()
                ))
            } else {
                Ok(serde_json::from_reader(file)?)
            }
        } else {
            Ok(Vec::new())
        }
    }

    pub fn write(onion_services: &[OnionService]) -> Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .mode(0600)
            .open(Self::filename())?;
        serde_json::to_writer_pretty(file, onion_services)?;
        Ok(())
    }
}

pub async fn get_onion_service(
    onion_address: Option<String>,
    listen_port: Option<u16>,
    service_port: Option<u16>,
    transient: bool,
    control_connection: &mut TorControlConnection,
) -> Result<OnionService, anyhow::Error> {
    let mut onion_services = if OnionServicesFile::exists() {
        match OnionServicesFile::read() {
            Ok(services) => services,
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "Error reading onion services file: {}",
                    error
                ))
            }
        }
    } else {
        Vec::new()
    };

    match onion_address.clone() {
        Some(onion_address) => {
            match onion_services
                .iter()
                .find(|&service| service.address == onion_address)
            {
                Some(onion_service) => Ok(onion_service.clone()),
                None => Err(anyhow::anyhow!(
                    "Onion address {} not found in services file",
                    onion_address
                )),
            }
        }
        None => {
            let listen_port = if listen_port.is_some() {
                listen_port.unwrap()
            } else {
                service_port.unwrap()
            };
            let service = control_connection
                .create_onion_service(
                    service_port.unwrap(),
                    &format!("127.0.0.1:{}", listen_port,),
                    transient,
                    None,
                )
                .await?;
            onion_services.push(service.clone());
            OnionServicesFile::write(&onion_services)?;
            Ok(service)
        }
    }
}

pub async fn test_onion_service_connection(
    listener: TcpListener,
    tor_proxy_address: &str,
    onion_service: &OnionService,
) -> Result<TcpListener, anyhow::Error> {
    println!("Testing onion service connection. Please be patient, this may take a few moments...");
    let handle = tokio::spawn(async move {
        match listener.accept().await {
            Ok(_) => Ok(listener),
            Err(error) => Err(error),
        }
    });
    let socket_addr = SocketAddr::from_str(tor_proxy_address)?;
    Socks5Stream::connect(socket_addr, onion_service.address.clone()).await?;

    Ok(handle.await??)
}
