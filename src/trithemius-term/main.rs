use crate::app::App;
use clap::{Args, Parser};
use std::str::FromStr;
use tor_client_lib::{
    auth::TorAuthentication,
    control_connection::{OnionAddress, OnionServiceListener, TorControlConnection},
};
use trithemius::engine::Engine;
use trithemius::logger::{Level, Logger, StandardLogger};
use trithemius::util::{get_onion_service, test_onion_service_connection};

mod app;
mod app_context;
mod commands;
mod input;
mod root;
mod term;
mod theme;

static SHORT_HELP: &str = "Trithemius - Anonymous, end-to-end encrypted chat";
static LONG_HELP: &str = "Trithemius - Anonymous, end-to-end encrypted chat

Uses Tor Onion Services to provide anonymization and NAT traversal.
To create an onion service, use the --create option, along with the --service-port.
You can also create a \"transient\" service by specifying the --transient flag
(this just means that the service will disappear when you disconnect).
You can re-use a previously-created non-transient onion service with the --onion-address flag.";

#[derive(Parser)]
#[command(author, version, about = SHORT_HELP, long_about = LONG_HELP)]
pub struct Cli {
    /// Tor control address
    #[arg(long, value_name = "ADDRESS", default_value_t = String::from("127.0.0.1:9051"))]
    tor_address: String,

    /// Tor proxy address
    #[arg(long, value_name = "ADDRESS", default_value_t = String::from("127.0.0.1:9050"))]
    tor_proxy_address: String,

    #[command(flatten)]
    onion_args: OnionArgs,

    /// Port for onion service
    #[arg(short, long, value_name = "PORT", conflicts_with = "onion_address")]
    service_port: Option<u16>,

    /// Local listen address (default is "127.0.0.1:<service_port>")
    #[arg(short, long, value_name = "PORT", conflicts_with = "onion_address")]
    listen_address: Option<String>,

    /// Create transient onion service (i.e., one that doesn't persist past a single session)
    #[arg(
        short,
        long,
        default_value_t = false,
        required = false,
        conflicts_with = "onion_address"
    )]
    transient: bool,

    /// Use debug logging
    #[arg(short, long, default_value_t = false)]
    debug: bool,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
pub struct OnionArgs {
    /// Create onion service. You'll need to specify at least --service-port as well
    #[arg(long, requires = "service_port")]
    create: bool,

    /// Onion address to (re-)use
    #[arg(long, value_name = "ONION_ADDRESS")]
    onion_address: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let mut control_connection = match TorControlConnection::connect(cli.tor_address.clone()).await
    {
        Ok(control_connection) => control_connection,
        Err(error) => {
            eprintln!("Error connecting to Tor control connection: {}", error);
            return;
        }
    };

    if let Err(error) = control_connection
        .authenticate(TorAuthentication::SafeCookie(None))
        .await
    {
        eprintln!("Error authenticating to Tor control connection: {}", error);
        return;
    }

    let service_port = if cli.onion_args.onion_address.is_some() {
        match OnionAddress::from_str(cli.onion_args.onion_address.as_ref().unwrap()) {
            Ok(onion_address) => onion_address.service_port(),
            Err(error) => {
                eprintln!(
                    "Error parsing onion address {}:, {}",
                    cli.onion_args.onion_address.unwrap(),
                    error
                );
                return;
            }
        }
    } else {
        cli.service_port.unwrap()
    };

    let (mut onion_service, listen_address) = match get_onion_service(
        cli.onion_args.onion_address,
        cli.listen_address,
        cli.service_port,
        cli.transient,
        &mut control_connection,
    )
    .await
    {
        Ok(onion_service) => {
            let listen_addresses = onion_service.listen_addresses_for_port(service_port);
            if listen_addresses.len() > 1 {
                eprintln!(
                    "Error: Got multiple listen addresses for onion_service {}",
                    onion_service.service_id(),
                );
                return;
            } else if listen_addresses.len() == 0 {
                eprintln!(
                    "Error: Found no listen addresses for onion_service {}",
                    onion_service.service_id(),
                );
                return;
            }
            (onion_service, listen_addresses[0].clone())
        }
        Err(error) => {
            eprintln!("Error getting onion service: {}", error);
            return;
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

    let listener = match test_onion_service_connection(
        listener,
        &cli.tor_proxy_address,
        &onion_service_address,
    )
    .await
    {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Error testing onion service connection: {}", error);
            return;
        }
    };

    let mut engine = match Engine::new(
        &mut onion_service,
        onion_service_address,
        &cli.tor_proxy_address,
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
