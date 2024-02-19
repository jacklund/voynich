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
    control_connection::{
        OnionAddress, OnionServiceListener, OnionServiceMapping, TorControlConnection,
        TorSocketAddr,
    },
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
    listen_address: &TorSocketAddr,
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

pub async fn create_persistent_onion_service(
    control_connection: &mut TorControlConnection,
    name: &str,
    service_port: u16,
    listen_address: &TorSocketAddr,
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

pub async fn use_persistent_onion_service(
    control_connection: &mut TorControlConnection,
    onion_service: &OnionService,
) -> Result<()> {
    let service_ids = match control_connection.get_info("onions/detached").await {
        Ok(service_ids) => service_ids,
        Err(error) => {
            return Err(anyhow!("{}", error));
        }
    };

    if !service_ids.contains(&onion_service.service_id().to_string()) {
        match control_connection
            .create_onion_service(
                onion_service.ports(),
                false,
                Some(onion_service.signing_key()),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(error) => Err(anyhow!("{}", error)),
        }
    } else {
        Ok(())
    }
}

pub async fn create_onion_service(
    control_connection: &mut TorControlConnection,
    onion_type: OnionType,
    service_port: Option<u16>,
    listen_address: Option<TorSocketAddr>,
) -> Result<(OnionService, OnionAddress, OnionServiceListener)> {
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
                None => TorSocketAddr::from_str(&format!("127.0.0.1:{}", service_port)).unwrap(),
            };
            let service =
                create_transient_onion_service(control_connection, service_port, &listen_address)
                    .await?;
            let listener = OnionServiceListener::bind(listen_address.clone()).await?;
            let onion_service_address =
                OnionAddress::new(service.service_id().clone(), service_port);
            Ok((service, onion_service_address, listener))
        }
        OnionType::Persistent { name, create } => {
            if create {
                let listen_address = match listen_address.clone() {
                    Some(listen_address) => listen_address,
                    None => {
                        TorSocketAddr::from_str(&format!("127.0.0.1:{}", service_port.unwrap()))
                            .unwrap()
                    }
                };
                let onion_service = create_persistent_onion_service(
                    control_connection,
                    &name,
                    service_port.unwrap(),
                    &listen_address,
                )
                .await?;
                let listener = OnionServiceListener::bind(listen_address.clone()).await?;
                let onion_service_address =
                    OnionAddress::new(onion_service.service_id().clone(), service_port.unwrap());
                Ok((onion_service, onion_service_address, listener))
            } else {
                let onion_address = get_onion_address(&name)?;
                let listen_address = match listen_address.clone() {
                    Some(listen_address) => listen_address,
                    None => TorSocketAddr::from_str(&format!(
                        "127.0.0.1:{}",
                        onion_address.service_port()
                    ))
                    .unwrap(),
                };
                let onion_service = get_onion_service(&name, &onion_address, &listen_address)?;
                use_persistent_onion_service(control_connection, &onion_service).await?;
                let listener = OnionServiceListener::bind(listen_address.clone()).await?;
                Ok((onion_service, onion_address, listener))
            }
        }
    }
}
