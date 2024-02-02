use crate::{
    app::App,
    cli::{Cli, OnionType},
};
use clap::Parser;
use std::str::FromStr;
use tor_client_lib::control_connection::{
    OnionAddress, OnionServiceListener, SocketAddr as OnionSocketAddr,
};
use voynich::config::get_config;
use voynich::control_connection::{
    create_permanent_onion_service, create_transient_onion_service, new_control_connection,
};
use voynich::engine::Engine;
use voynich::logger::{Level, Logger, StandardLogger};
use voynich::util::{get_onion_address, get_onion_service, test_onion_service_connection};

mod app;
mod app_context;
mod cli;
mod commands;
mod input;
mod root;
mod term;
mod theme;
mod widgets;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let config = match get_config(None) {
        Ok(config) => config.update((&cli).into()),
        Err(error) => {
            eprintln!("Error reading configuration: {}", error);
            return;
        }
    };

    let mut control_connection = match new_control_connection(
        config.tor.control_address.unwrap(),
        config.tor.authentication,
        config.tor.hashed_password,
        config.tor.cookie,
    )
    .await
    {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("Error connecting to Tor control connection: {}", error);
            return;
        }
    };

    let (mut onion_service, service_port, listen_address) = match cli.onion_type {
        OnionType::Transient => {
            let service_port = match cli.service_port {
                Some(port) => port,
                None => {
                    eprintln!("Error: No service port specified for transient onion service");
                    return;
                }
            };
            let listen_address = match cli.listen_address {
                Some(listen_address) => listen_address,
                None => OnionSocketAddr::from_str(&format!("127.0.0.1:{}", service_port)).unwrap(),
            };
            let service = match create_transient_onion_service(
                &mut control_connection,
                service_port,
                &listen_address,
            )
            .await
            {
                Ok(service) => service,
                Err(error) => {
                    eprintln!("Error creating transient onion service: {}", error);
                    return;
                }
            };
            (service, service_port, listen_address)
        }
        OnionType::Permanent => {
            let name = match cli.name {
                Some(name) => name,
                None => {
                    eprintln!("'--name' must be specified with '--onion-type permanent'");
                    return;
                }
            };
            if cli.create {
                let listen_address = match cli.listen_address {
                    Some(listen_address) => listen_address,
                    None => OnionSocketAddr::from_str(&format!(
                        "127.0.0.1:{}",
                        cli.service_port.unwrap()
                    ))
                    .unwrap(),
                };
                let onion_service = match create_permanent_onion_service(
                    &mut control_connection,
                    &name,
                    cli.service_port.unwrap(),
                    &listen_address,
                )
                .await
                {
                    Ok(service) => service,
                    Err(error) => {
                        eprintln!("Error creating onion service: {}", error);
                        return;
                    }
                };
                (onion_service, cli.service_port.unwrap(), listen_address)
            } else {
                let onion_address = match get_onion_address(&name) {
                    Ok(address) => address,
                    Err(error) => {
                        eprintln!("Error getting onion address: {}", error);
                        return;
                    }
                };
                let listen_address = match cli.listen_address {
                    Some(listen_address) => listen_address,
                    None => OnionSocketAddr::from_str(&format!(
                        "127.0.0.1:{}",
                        onion_address.service_port()
                    ))
                    .unwrap(),
                };
                let onion_service = match get_onion_service(&name, &onion_address, &listen_address)
                {
                    Ok(service) => service,
                    Err(error) => {
                        eprintln!("Error getting onion service: {}", error);
                        return;
                    }
                };
                (onion_service, onion_address.service_port(), listen_address)
            }
        }
    };

    let mut logger = StandardLogger::new(500);
    if cli.debug {
        logger.set_log_level(Level::Debug);
    }

    let listener = match OnionServiceListener::bind(listen_address.clone()).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Error binding to address {}: {}", listen_address, error);
            return;
        }
    };

    let onion_service_address = OnionAddress::new(onion_service.service_id().clone(), service_port);

    let listener = if cli.no_connection_test {
        listener
    } else {
        match test_onion_service_connection(
            listener,
            &config.tor.proxy_address.clone().unwrap(),
            &onion_service_address,
        )
        .await
        {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("Error testing onion service connection: {}", error);
                return;
            }
        }
    };

    let mut engine = match Engine::new(
        &mut onion_service,
        onion_service_address,
        &config.tor.proxy_address.unwrap(),
        cli.debug,
    )
    .await
    {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("Error creating engine: {}", error);
            return;
        }
    };
    let _ = App::run(&mut engine, &listener, &mut logger).await;
}
