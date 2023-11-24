use anyhow::Result;
use clap::crate_name;
use lazy_static::lazy_static;
use std::env;
use std::fs::File;
use std::path::Path;
use std::{net::SocketAddr, str::FromStr};
use tokio::net::TcpListener;
use tokio_socks::tcp::Socks5Stream;
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
        let file = File::open(Self::filename())?;
        Ok(serde_json::from_reader(file)?)
    }

    pub fn write(onion_services: &[OnionService]) -> Result<()> {
        let file = File::create(Self::filename())?;
        serde_json::to_writer_pretty(file, onion_services)?;
        Ok(())
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
