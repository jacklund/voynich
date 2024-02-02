use clap::{Args, Parser, ValueEnum};
use tor_client_lib::control_connection::SocketAddr;
use voynich::config::{Config, TorAuthConfig};

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
    pub tor_address: Option<String>,

    /// Tor proxy address - default is 127.0.0.1:9050
    #[arg(long, value_name = "ADDRESS")]
    pub tor_proxy_address: Option<String>,

    /// Listen address to use for onion service
    /// Default is "127.0.0.1:<service-port>"
    /// You may need to specify this for permanent services which have multiple listen addresses
    #[arg(long, value_name = "LOCAL-ADDRESS", required = true)]
    pub listen_address: Option<SocketAddr>,

    /// Service port to use for the transient or newly created persistent onion service
    #[arg(long, required_if_eq_any([("onion_type", "transient"), ("create", "true")]))]
    pub service_port: Option<u16>,

    /// Tor Authentication Arguments
    #[command(flatten)]
    pub auth_args: AuthArgs,

    /// Don't run connection test on startup (by default, it will run the test)
    #[arg(long, default_value_t = false)]
    pub no_connection_test: bool,

    /// Use debug logging
    #[arg(short, long, default_value_t = false)]
    pub debug: bool,

    /// Type of the onion service
    #[arg(short, long, value_enum)]
    pub onion_type: OnionType,

    /// Create the onion service.
    /// Ignored if --onion-type is "transient"
    #[arg(long, default_value_t = false)]
    pub create: bool,

    /// Name of the onion service. Ignored if onion type is "transient"
    ///
    /// If --create is specified, saves the created service under that name.
    /// If not, it tries to look up a saved onion service by that name
    #[arg(short, long)]
    pub name: Option<String>,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum OnionType {
    Transient,
    Permanent,
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
        config.system.debug = cli.debug;
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
                config.tor.cookie = Some(cookie.as_bytes().to_vec());
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
