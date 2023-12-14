use crate::tor::onion_service::find_static_onion_service;
use clap::{Args, Parser};
use tokio;
use trithemius::onion_service::OnionService;

mod tor;

static SHORT_HELP: &str = "Import keys into Trithemius";
static LONG_HELP: &str = "Import keys into Trithemius

Import a private or public key into Trithemius.
Private keys are used for generating onion services.
Public keys are for identifying incoming chat connections.";

#[derive(Parser)]
#[command(author, version, about = SHORT_HELP, long_about = LONG_HELP)]
pub struct Cli {
    /// Key type to import
    #[command(flatten)]
    key_type: KeyType,

    /// Import private key from Tor Onion Service configuration
    #[arg(long, value_name = "SERVICE_NAME", conflicts_with = "public_key")]
    onion_service: Option<String>,

    /// Import public key from Keyoxide (for contacts)
    #[arg(long, value_name = "EMAIL_ADDRESS", conflicts_with = "private_key")]
    keyoxide: Option<String>,

    /// Import private or public key from GPG keyring
    #[arg(long, value_name = "EMAIL_ADDRESS")]
    gpg: Option<String>,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
pub struct KeyType {
    /// Import private key
    #[arg(long, default_value_t = false)]
    private_key: bool,

    /// Import public key
    #[arg(long, default_value_t = false)]
    public_key: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Some(service_name) = cli.onion_service {
        let onion_service = match find_static_onion_service(&service_name) {
            Ok(Some(onion_service)) => onion_service,
            Ok(None) => {
                eprintln!("No onion service named '{}' found", service_name);
                return;
            }
            Err(error) => {
                eprintln!("Error finding onion service '{}': {}", service_name, error);
                return;
            }
        };
        println!("{:?}", onion_service);
        println!(
            "Verifying key from secret key = {:?}",
            onion_service.secret_key().verifying_key().as_bytes()
        );
        println!(
            "Verifying key = {:?}",
            onion_service.public_key().as_bytes()
        );
        println!(
            "Verifying keys are identical is {}",
            onion_service.secret_key().verifying_key().as_bytes()
                == onion_service.public_key().as_bytes()
        );
    }

    if let Some(_email) = cli.keyoxide {
        unimplemented!()
    }

    if let Some(_email) = cli.gpg {
        unimplemented!()
    }
}
