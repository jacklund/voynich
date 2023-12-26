use clap::Parser;
use trithemius::torrc::find_torrc_onion_service;
use trithemius::{
    onion_service::OnionService, onion_service_data::OnionServiceData, util::OnionServicesFile,
};

static SHORT_HELP: &str = "Import existing onion services into Trithemius";
static LONG_HELP: &str = "Import existing onion services into Trithemius

Imports onion services set up in the torrc into trithemius

Typical usage: % sudo parse_onion_service --name foo | import_onion_service";

#[derive(Parser)]
#[command(author, version, about = SHORT_HELP, long_about = LONG_HELP)]
pub struct Cli {}

#[tokio::main]
async fn main() {
    let info: OnionServiceData = match serde_json::from_reader(std::io::stdin()) {
        Ok(info) => info,
        Err(error) => {
            eprintln!("Error parsing JSON: {}", error);
            return;
        }
    };

    match find_torrc_onion_service(&info.name) {
        Ok(Some(onion_service_info)) => {
            let onion_service = OnionService::new(
                info.name,
                onion_service_info.ports().clone(),
                info.hostname,
                info.secret_key,
            );
            let mut onion_services = match OnionServicesFile::read() {
                Ok(services) => services,
                Err(error) => {
                    eprintln!("Error reading {}: {}", OnionServicesFile::filename(), error);
                    return;
                }
            };
            onion_services.push(onion_service);
            onion_services.sort();
            onion_services.dedup();
            if let Err(error) = OnionServicesFile::write(&onion_services) {
                eprintln!("Error writing {}: {}", OnionServicesFile::filename(), error);
            }
        }
        Ok(None) => {
            eprintln!("Onion service '{}' not found", info.name);
        }
        Err(error) => {
            eprintln!("Error looking for onion service {}: {}", info.name, error);
        }
    }
}
