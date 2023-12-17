use clap::Parser;
use std::str::FromStr;
use tor_client_lib::TorEd25519SigningKey;
use trithemius::torrc::find_torrc_onion_service;
use trithemius::{onion_service::OnionService, util::OnionServicesFile};

static SHORT_HELP: &str = "Import existing onion services into Trithemius";
static LONG_HELP: &str = "Import existing onion services into Trithemius

Imports onion services set up in the torrc into trithemius";

// import-onion-service --name foo --hostname bar --signing-key abcd
#[derive(Parser)]
#[command(author, version, about = SHORT_HELP, long_about = LONG_HELP)]
pub struct Cli {
    #[arg(long, required = true)]
    name: String,

    #[arg(long, required = true)]
    hostname: Option<String>,

    #[arg(long, required = true)]
    signing_key: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match find_torrc_onion_service(&cli.name) {
        Ok(Some(onion_service_info)) => {
            let key = match TorEd25519SigningKey::from_str(&cli.signing_key.unwrap()) {
                Ok(key) => key,
                Err(error) => {
                    eprintln!("Error deserializing signing key: {}", error);
                    return;
                }
            };
            let onion_service = OnionService::new(
                cli.name,
                onion_service_info.ports().clone(),
                cli.hostname.unwrap(),
                key,
            );
            let mut onion_services = match OnionServicesFile::read() {
                Ok(services) => services,
                Err(error) => {
                    eprintln!("Error reading {}: {}", OnionServicesFile::filename(), error);
                    return;
                }
            };
            onion_services.push(onion_service);
            if let Err(error) = OnionServicesFile::write(&onion_services) {
                eprintln!("Error writing {}: {}", OnionServicesFile::filename(), error);
            }
        }
        Ok(None) => {
            eprintln!("Onion service '{}' not found", cli.name);
        }
        Err(error) => {
            eprintln!("Error looking for onion service {}: {}", cli.name, error);
        }
    }
}
