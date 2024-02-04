use crate::config::TorAuthConfig;
use crate::onion_service::{OnionService, OnionType};
use crate::util::{get_onion_address, get_onion_service, save_onion_service};
use anyhow::{anyhow, Result};
use rpassword::read_password;
use std::io::Write;
use std::str::FromStr;
use tokio::net::ToSocketAddrs;
use tor_client_lib::{
    auth::TorAuthentication,
    control_connection::{OnionServiceMapping, SocketAddr, TorControlConnection},
};

pub async fn connect_to_tor<A: ToSocketAddrs>(
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
        Ok(service) => {
            let onion_service = OnionService::new(name, service);
            save_onion_service(&onion_service, service_port)?;
            Ok(onion_service)
        }
        Err(error) => Err(anyhow!("{}", error)),
    }
}

pub async fn create_onion_service(
    control_connection: &mut TorControlConnection,
    name: Option<String>,
    create: bool,
    onion_type: OnionType,
    service_port: Option<u16>,
    listen_address: Option<SocketAddr>,
) -> Result<(OnionService, u16, SocketAddr)> {
    match onion_type {
        OnionType::Transient => {
            let service_port = match service_port {
                Some(port) => port,
                None => {
                    return Err(anyhow!(
                        "Error: No service port specified for transient onion service"
                    ));
                }
            };
            let listen_address = match listen_address.clone() {
                Some(listen_address) => listen_address,
                None => SocketAddr::from_str(&format!("127.0.0.1:{}", service_port)).unwrap(),
            };
            let service =
                create_transient_onion_service(control_connection, service_port, &listen_address)
                    .await?;
            Ok((service, service_port, listen_address))
        }
        OnionType::Permanent => {
            let name = match name.clone() {
                Some(name) => name,
                None => {
                    return Err(anyhow!(
                        "'--name' must be specified with '--onion-type permanent'"
                    ));
                }
            };
            if create {
                let listen_address = match listen_address.clone() {
                    Some(listen_address) => listen_address,
                    None => SocketAddr::from_str(&format!("127.0.0.1:{}", service_port.unwrap()))
                        .unwrap(),
                };
                let onion_service = create_permanent_onion_service(
                    control_connection,
                    &name,
                    service_port.unwrap(),
                    &listen_address,
                )
                .await?;
                Ok((onion_service, service_port.unwrap(), listen_address))
            } else {
                let onion_address = get_onion_address(&name)?;
                let listen_address = match listen_address.clone() {
                    Some(listen_address) => listen_address,
                    None => {
                        SocketAddr::from_str(&format!("127.0.0.1:{}", onion_address.service_port()))
                            .unwrap()
                    }
                };
                let onion_service = get_onion_service(&name, &onion_address, &listen_address)?;
                Ok((onion_service, onion_address.service_port(), listen_address))
            }
        }
    }
}
