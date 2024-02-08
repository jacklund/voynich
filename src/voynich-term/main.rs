use crate::{app::App, cli::Cli};
use clap::Parser;
use tor_client_lib::control_connection::{OnionAddress, OnionServiceListener};
use voynich::config::get_config;
use voynich::control_connection::{connect_to_tor, create_onion_service};
use voynich::engine::Engine;
use voynich::logger::{Level, Logger, StandardLogger};
use voynich::util::test_onion_service_connection;

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
    // Parse the CLI
    let cli = Cli::parse();
    let config = match get_config(None) {
        Ok(config) => config.update((&cli).into()),
        Err(error) => {
            eprintln!("Error reading configuration: {}", error);
            return;
        }
    };

    // Logging
    let mut logger = StandardLogger::new(500);
    if config.system.debug {
        logger.set_log_level(Level::Debug);
    }

    // Get a connection to Tor
    let mut control_connection = match connect_to_tor(
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

    // Create our onion service
    let (mut onion_service, service_port, listen_address) = match create_onion_service(
        &mut control_connection,
        cli.name,
        cli.create,
        cli.onion_type,
        cli.service_port,
        cli.listen_address,
    )
    .await
    {
        Ok((onion_service, service_port, listen_address)) => {
            (onion_service, service_port, listen_address)
        }
        Err(error) => {
            eprintln!("Error creating onion service: {}", error);
            return;
        }
    };

    // Create the listener
    let listener = match OnionServiceListener::bind(listen_address.clone()).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Error binding to address {}: {}", listen_address, error);
            return;
        }
    };

    // Get our onion address
    let onion_service_address = OnionAddress::new(onion_service.service_id().clone(), service_port);

    // Get our listener
    let listener = if !config.system.connection_test {
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

    // Set up the engine
    let mut engine = match Engine::new(
        &mut onion_service,
        onion_service_address,
        &config.tor.proxy_address.unwrap(),
        config.system.debug,
    )
    .await
    {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("Error creating engine: {}", error);
            return;
        }
    };

    // Start 'er up
    let _ = App::run(&mut engine, &listener, &mut logger).await;
}
