use crate::app::App;
use clap::{Args, Parser};
use rpassword::read_password;
use std::io::Write;
use std::str::FromStr;
use tor_client_lib::{
    auth::TorAuthentication,
    control_connection::{
        OnionAddress, OnionServiceListener, OnionServiceMapping, SocketAddr as OnionSocketAddr,
        TorControlConnection,
    },
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
mod widgets;

static SHORT_HELP: &str = "Trithemius - Anonymous, end-to-end encrypted chat";
static LONG_HELP: &str = "Trithemius - Anonymous, end-to-end encrypted chat

Uses Tor Onion Services to provide anonymization and NAT traversal.
To create an onion service, use the --create option, along with the --service-port.
You can also create a \"transient\" service by specifying the --transient flag
(this just means that the service will disappear when you disconnect).
You can re-use a previously-created non-transient onion service with the --onion-address flag.";

#[derive(Debug, Parser)]
#[command(author, version, about = SHORT_HELP, long_about = LONG_HELP)]
pub struct Cli {
    /// Tor control address - default is 127.0.0.1:9051
    #[arg(long, value_name = "ADDRESS")]
    tor_address: Option<String>,

    /// Tor proxy address - default is 127.0.0.1:9050
    #[arg(long, value_name = "ADDRESS")]
    tor_proxy_address: Option<String>,

    #[command(flatten)]
    onion_args: OnionArgs,

    /// Listen address to use for onion service
    /// Default is "127.0.0.1:<service-port>"
    /// You may need to specify this for permanent services which have multiple listen addresses
    #[arg(long, value_name = "LOCAL-ADDRESS")]
    listen_address: Option<OnionSocketAddr>,

    /// Tor Authentication Arguments
    #[command(flatten)]
    auth_args: AuthArgs,

    /// Don't run connection test on startup (by default, it will run the test)
    #[arg(long, default_value_t = false)]
    no_connection_test: bool,

    /// Use debug logging
    #[arg(short, long, default_value_t = false)]
    debug: bool,
}

#[derive(Args, Debug)]
#[group(required = true, multiple = false)]
pub struct OnionArgs {
    /// Create a transient onion service
    #[arg(long, value_name = "SERVICE-PORT")]
    transient: Option<u16>,

    /// Use a permanent onion service
    #[arg(long, value_name = "ONION-HOST:SERVICE-PORT")]
    permanent: Option<OnionAddress>,
}

#[derive(Args, Clone, Debug)]
#[group(required = false, multiple = false)]
pub struct AuthArgs {
    /// Tor service authentication. Set the value of the cookie; no value means use the cookie from the cookie file
    #[arg(long = "safe-cookie")]
    safe_cookie: Option<Option<String>>,

    /// Tor service authentication. Set the value of the password; no value means prompt for the password
    #[arg(long = "hashed-password")]
    hashed_password: Option<Option<String>>,
}

impl From<&Cli> for Config {
    fn from(cli: &Cli) -> Config {
        let mut config = Config::default();
        config.logging.debug = cli.debug;
        config.tor.proxy_address = cli.tor_proxy_address.clone();
        config.tor.control_address = cli.tor_address.clone();
        match &cli.auth_args {
            AuthArgs {
                safe_cookie: None,
                hashed_password: None,
            } => {}
            AuthArgs {
                safe_cookie: Some(None),
                hashed_password: None,
            } => {
                config.tor.authentication = Some(TorAuthConfig::SafeCookie);
            }
            AuthArgs {
                safe_cookie: Some(Some(cookie)),
                hashed_password: None,
            } => {
                config.tor.authentication = Some(TorAuthConfig::SafeCookie);
                config.tor.cookie = Some(cookie.clone());
            }
            AuthArgs {
                safe_cookie: None,
                hashed_password: Some(None),
            } => {
                config.tor.authentication = Some(TorAuthConfig::HashedPassword);
            }
            AuthArgs {
                safe_cookie: None,
                hashed_password: Some(Some(password)),
            } => {
                config.tor.authentication = Some(TorAuthConfig::HashedPassword);
                config.tor.hashed_password = Some(password.clone());
            }
            _ => {
                unreachable!()
            }
        }

        // TODO: Figure out onion service configs

        config
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
        Some(TorAuthConfig::HashedPassword) => match config.tor.hashed_password {
            Some(password) => TorAuthentication::HashedPassword(password),
            None => {
                print!("Type a password: ");
                std::io::stdout().flush().unwrap();
                let password = read_password().unwrap();
                TorAuthentication::HashedPassword(password)
            }
        },
        Some(TorAuthConfig::SafeCookie) => match config.tor.cookie {
            Some(cookie) => TorAuthentication::SafeCookie(Some(cookie.as_bytes().to_vec())),
            None => TorAuthentication::SafeCookie(None),
        },
        None => TorAuthentication::Null,
    };

    if let Err(error) = control_connection.authenticate(tor_authentication).await {
        eprintln!("Error authenticating to Tor control connection: {}", error);
        return;
    }

    let (mut onion_service, service_port, listen_address) = match cli.onion_args {
        OnionArgs {
            transient: Some(service_port),
            permanent: None,
        } => {
            let listen_address = match cli.listen_address {
                Some(listen_address) => listen_address,
                None => OnionSocketAddr::from_str(&format!("127.0.0.1:{}", service_port)).unwrap(),
            };
            let service: OnionService = match control_connection
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
                Ok(service) => service.into(),
                Err(error) => {
                    eprintln!("Error creating onion service: {}", error);
                    return;
                }
            };
            (service, service_port, listen_address)
        }
        OnionArgs {
            transient: None,
            permanent: Some(onion_address),
        } => {
            let listen_address = match cli.listen_address {
                Some(listen_address) => listen_address,
                None => OnionSocketAddr::from_str(&format!(
                    "127.0.0.1:{}",
                    onion_address.service_port()
                ))
                .unwrap(),
            };
            match get_onion_service(&onion_address).await {
                Ok(onion_service) => {
                    match onion_service
                        .ports()
                        .iter()
                        .find(|m| *m.listen_address() == listen_address)
                    {
                        Some(onion_service_mapping) => (
                            onion_service.clone(),
                            onion_service_mapping.virt_port(),
                            listen_address,
                        ),
                        None => {
                            eprintln!(
                                "Listen address {} not found for service {}",
                                listen_address, onion_address,
                            );
                            return;
                        }
                    }
                }
                Err(error) => {
                    eprintln!("Error getting onion service: {}", error);
                    return;
                }
            }
        }
        _ => {
            unreachable!()
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
