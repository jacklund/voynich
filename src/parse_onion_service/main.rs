use clap::Parser;
use voynich::onion_service_data::find_static_onion_service;

static SHORT_HELP: &str = "Parse files for existing onion service";
static LONG_HELP: &str = "Parse files for existing onion service

Read files for the named permanent onion service and print output.
Must be run as root. Used in conjunction with import-onion-service";

// import-onion-service --name foo --hostname bar --signing-key abcd
#[derive(Parser)]
#[command(author, version, about = SHORT_HELP, long_about = LONG_HELP)]
pub struct Cli {
    #[arg(long, required = true)]
    name: String,
}

fn main() {
    let cli = Cli::parse();

    let info = match find_static_onion_service(&cli.name) {
        Ok(Some(onion_service)) => onion_service,
        Ok(None) => {
            eprintln!("Onion service {} not found", cli.name);
            return;
        }
        Err(error) => {
            eprintln!("Error reading onion service {}: {}", cli.name, error);
            return;
        }
    };

    match serde_json::to_string_pretty(&info) {
        Ok(json) => println!("{}", json),
        Err(error) => eprintln!("Error serializing info as JSON: {}", error),
    }
}
