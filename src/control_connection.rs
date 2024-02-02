use crate::config::TorAuthConfig;
use crate::onion_service::OnionService;
use anyhow::{anyhow, Result};
use rpassword::read_password;
use std::io::Write;
use tokio::net::ToSocketAddrs;
use tor_client_lib::{
    auth::TorAuthentication,
    control_connection::{OnionServiceMapping, SocketAddr, TorControlConnection},
};

pub async fn new_control_connection<A: ToSocketAddrs>(
    control_address: A,
    authentication: Option<TorAuthConfig>,
    hashed_password: Option<String>,
    cookie: Option<Vec<u8>>,
) -> Result<TorControlConnection> {
    let mut control_connection = match TorControlConnection::connect(control_address).await {
        Ok(control_connection) => control_connection,
        Err(error) => {
            return Err(anyhow!(
                "Error connecting to Tor control connection: {}",
                error
            ));
        }
    };

    let tor_authentication = match authentication {
        Some(TorAuthConfig::HashedPassword) => match hashed_password {
            Some(password) => TorAuthentication::HashedPassword(password),
            None => {
                print!("Type a password: ");
                std::io::stdout().flush().unwrap();
                let password = read_password().unwrap();
                TorAuthentication::HashedPassword(password)
            }
        },
        Some(TorAuthConfig::SafeCookie) => match cookie {
            Some(cookie) => TorAuthentication::SafeCookie(Some(cookie)),
            None => TorAuthentication::SafeCookie(None),
        },
        None => TorAuthentication::Null,
    };

    if let Err(error) = control_connection.authenticate(tor_authentication).await {
        return Err(anyhow!(
            "Error authenticating to Tor control connection: {}",
            error
        ));
    }

    Ok(control_connection)
}

pub async fn create_transient_onion_service(
    control_connection: &mut TorControlConnection,
    service_port: u16,
    listen_address: &SocketAddr,
) -> Result<OnionService> {
    match control_connection
        .create_onion_service(
            &[OnionServiceMapping::new(
                service_port,
                Some(listen_address.clone()),
            )],
            true,
            None,
        )
        .await
    {
        Ok(service) => Ok(service.into()),
        Err(error) => Err(anyhow!("{}", error)),
    }
}

pub async fn create_permanent_onion_service(
    control_connection: &mut TorControlConnection,
    name: &str,
    service_port: u16,
    listen_address: &SocketAddr,
) -> Result<OnionService> {
    match control_connection
        .create_onion_service(
            &[OnionServiceMapping::new(
                service_port,
                Some(listen_address.clone()),
            )],
            false,
            None,
        )
        .await
    {
        Ok(service) => Ok(OnionService::new(name, service)),
        Err(error) => Err(anyhow!("{}", error)),
    }
}
