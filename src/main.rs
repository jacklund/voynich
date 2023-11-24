use crate::app::App;
use crate::engine::Engine;
use crate::logger::{Level, Logger, StandardLogger};
use crate::util::{test_onion_service_connection, OnionServicesFile};
use clap::{Args, Parser};
use tokio::net::TcpListener;
use tor_client_lib::{
    auth::TorAuthentication, control_connection::TorControlConnection, OnionService,
};

mod app;
mod app_context;
mod chat;
mod commands;
mod crypto;
mod engine;
mod input;
mod logger;
mod root;
mod term;
mod theme;
mod util;

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

    /// Local listen port (default is same as --service-port)
    #[arg(short, long, value_name = "PORT", conflicts_with = "onion_address")]
    listen_port: Option<u16>,

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

async fn get_onion_service(
    cli: &Cli,
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

    match cli.onion_args.onion_address.clone() {
        Some(onion_address) => {
            match onion_services
                .iter()
                .find(|&service| service.address == onion_address)
            {
                Some(onion_service) => Ok(onion_service.clone()),
                None => {
                    return Err(anyhow::anyhow!(
                        "Onion address {} not found in services file",
                        onion_address
                    ))
                }
            }
        }
        None => {
            let listen_port = if cli.listen_port.is_some() {
                cli.listen_port.unwrap()
            } else {
                cli.service_port.unwrap()
            };
            let service = control_connection
                .create_onion_service(
                    cli.service_port.unwrap(),
                    &format!("127.0.0.1:{}", listen_port,),
                    cli.transient,
                    None,
                )
                .await?;
            onion_services.push(service.clone());
            OnionServicesFile::write(&onion_services)?;
            Ok(service)
        }
    }
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

    let mut onion_service = match get_onion_service(&cli, &mut control_connection).await {
        Ok(onion_service) => onion_service,
        Err(error) => {
            eprintln!("Error getting onion service: {}", error);
            return;
        }
    };

    let mut logger = StandardLogger::new(500);
    if cli.debug {
        logger.set_log_level(Level::Debug);
    }

    let listener = match TcpListener::bind(&format!(
        "127.0.0.1:{}",
        onion_service.listen_address.port()
    ))
    .await
    {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!(
                "Error binding to port {}: {}",
                onion_service.listen_address.port(),
                error
            );
            return;
        }
    };

    let listener =
        match test_onion_service_connection(listener, &cli.tor_proxy_address, &onion_service).await
        {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("Error testing onion service connection: {}", error);
                return;
            }
        };

    let mut engine = match Engine::new(&mut onion_service, &cli.tor_proxy_address, cli.debug).await
    {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("Error creating engine: {}", error);
            return;
        }
    };
    let _ = App::run(&mut engine, &listener, &mut logger).await;
}
