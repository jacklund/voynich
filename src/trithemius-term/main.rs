use crate::app::App;
use anyhow::{anyhow, Result};
use clap::{Args, Parser};
use rpassword::read_password;
use std::io::Write;
use std::str::FromStr;
use tor_client_lib::{
    auth::TorAuthentication,
    control_connection::{OnionAddress, OnionServiceListener, SocketAddr, TorControlConnection},
};
use trithemius::config::{get_config, Config, TorAuthConfig};
use trithemius::engine::Engine;
use trithemius::logger::{Level, Logger, StandardLogger};
use trithemius::onion_service::OnionService;
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
    /// Tor control address - default is 127.0.0.1:9051
    #[arg(long, value_name = "ADDRESS")]
    tor_address: Option<String>,

    /// Tor proxy address - default is 127.0.0.1:9050
    #[arg(long, value_name = "ADDRESS")]
    tor_proxy_address: Option<String>,

    /// How to authenticate to Tor - default is no authentication
    #[arg(value_enum, long, short)]
    authentication: Option<TorAuthConfig>,

    #[command(flatten)]
    onion_args: OnionArgs,

    /// Port for onion service
    #[arg(short, long, value_name = "PORT", conflicts_with = "onion_address")]
    service_port: Option<u16>,

    /// Local listen address (default is "127.0.0.1:<service_port>")
    #[arg(short, long, value_name = "HOST:PORT")]
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

impl From<&Cli> for Config {
    fn from(cli: &Cli) -> Config {
        let mut config = Config::default();
        config.logging.debug = cli.debug;
        config.tor.proxy_address = cli.tor_proxy_address.clone();
        config.tor.control_address = cli.tor_address.clone();
        config.tor.authentication = cli.authentication.clone();
        // TODO: Figure out onion service configs

        config
    }
}

fn find_listen_address(
    cli: &Cli,
    onion_service: &OnionService,
    listen_addresses: &[SocketAddr],
) -> Result<SocketAddr> {
    if listen_addresses.len() > 1 {
        if let Some(listen_address) = &cli.listen_address {
            let listen_address = match SocketAddr::from_str(listen_address) {
                Ok(address) => address,
                Err(error) => {
                    return Err(anyhow!(
                        "Error parsing listen address {}: {}",
                        listen_address,
                        error
                    ));
                }
            };
            match listen_addresses.iter().find(|l| **l == listen_address) {
                Some(listen_address) => Ok(listen_address.clone()),
                None => Err(anyhow!(
                    "Listen address {} not found in service {}",
                    cli.listen_address.as_ref().unwrap(),
                    onion_service.name()
                )),
            }
        } else {
            Err(anyhow!(
                "Error: Got multiple listen addresses for onion_service {}",
                onion_service.service_id(),
            ))
        }
    } else if listen_addresses.is_empty() {
        Err(anyhow!(
            "Error: Found no listen addresses for onion_service {}",
            onion_service.service_id(),
        ))
    } else {
        Ok(listen_addresses[0].clone())
    }
}

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

    let mut control_connection =
        match TorControlConnection::connect(config.tor.control_address.unwrap()).await {
            Ok(control_connection) => control_connection,
            Err(error) => {
                eprintln!("Error connecting to Tor control connection: {}", error);
                return;
            }
        };

    let tor_authentication = match config.tor.authentication {
        Some(TorAuthConfig::HashedPassword) => {
            print!("Type a password: ");
            std::io::stdout().flush().unwrap();
            let password = read_password().unwrap();
            TorAuthentication::HashedPassword(password)
        }
        Some(TorAuthConfig::SafeCookie) => {
            print!("Type the cookie <leave empty to read cookie file directly>: ");
            std::io::stdout().flush().unwrap();
            let cookie = read_password().unwrap();
            if cookie.is_empty() {
                TorAuthentication::SafeCookie(None)
            } else {
                TorAuthentication::SafeCookie(Some(cookie.as_bytes().to_vec()))
            }
        }
        None => TorAuthentication::Null,
    };

    if let Err(error) = control_connection.authenticate(tor_authentication).await {
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
        cli.onion_args.onion_address.clone(),
        cli.listen_address.clone(),
        cli.service_port,
        cli.transient,
        &mut control_connection,
    )
    .await
    {
        Ok(onion_service) => {
            let listen_addresses = onion_service.listen_addresses_for_port(service_port);
            let listen_address = match find_listen_address(&cli, &onion_service, &listen_addresses)
            {
                Ok(listen_address) => listen_address,
                Err(error) => {
                    eprintln!("Error: {}", error);
                    return;
                }
            };
            (onion_service, listen_address)
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
