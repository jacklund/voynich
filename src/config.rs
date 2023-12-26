use anyhow::Result;
use clap::ValueEnum;
use lazy_static::lazy_static;
use serde::Deserialize;
use std::env;
use std::fs::read_to_string;
use std::io::ErrorKind;

lazy_static! {
    pub static ref HOME: String = match env::var("HOME") {
        Ok(value) => value,
        Err(error) => {
            panic!("Error finding home directory: {}", error);
        }
    };
    pub static ref CONFIG_HOME: String = match env::var("XDG_CONFIG_HOME") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => format!("{}/.config", *HOME),
        Err(error) => {
            panic!("Error getting value of XDG_CONFIG_HOME: {}", error);
        }
    };
}

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    pub logging: LoggingConfig,
    pub tor: TorConfig,
    pub onion_services: Vec<OnionServiceConfig>,
}

impl Config {
    pub fn new() -> Self {
        Self {
            logging: LoggingConfig::default(),
            tor: TorConfig::new(),
            onion_services: Vec::new(),
        }
    }

    pub fn update(self, other: Config) -> Self {
        Self {
            logging: self.logging.update(other.logging),
            tor: self.tor.update(other.tor),
            onion_services: other.onion_services,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct LoggingConfig {
    pub debug: bool,
}

impl LoggingConfig {
    pub fn update(self, other: LoggingConfig) -> Self {
        Self { debug: other.debug }
    }
}

#[derive(Clone, Debug, Deserialize, ValueEnum)]
pub enum TorAuthConfig {
    #[serde(alias = "hashed-password")]
    HashedPassword,

    #[serde(alias = "safe-cookie")]
    SafeCookie,
}

#[derive(Debug, Default, Deserialize)]
pub struct TorConfig {
    pub proxy_address: Option<String>,
    pub control_address: Option<String>,
    pub authentication: Option<TorAuthConfig>,
    pub cookie: Option<String>,
    pub hashed_password: Option<String>,
}

impl TorConfig {
    pub fn new() -> Self {
        Self {
            proxy_address: Some("127.0.0.1:9050".to_string()),
            control_address: Some("127.0.0.1:9051".to_string()),
            authentication: None,
            cookie: None,
            hashed_password: None,
        }
    }

    pub fn update(self, other: TorConfig) -> Self {
        Self {
            proxy_address: other.proxy_address.or(self.proxy_address),
            control_address: other.control_address.or(self.control_address),
            authentication: other.authentication.or(self.authentication),
            cookie: other.cookie.or(self.cookie),
            hashed_password: other.hashed_password.or(self.hashed_password),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct OnionServiceConfig {
    pub name: String,
    pub onion_address: Option<String>,
    pub transient: Option<bool>,
    pub service_port: Option<u16>,
    pub listen_address: Option<String>,
}

pub fn read_config_file(location: Option<String>) -> Result<Option<Config>> {
    let location = match location {
        Some(location) => location,
        None => format!("{}/trithemius/config.toml", *CONFIG_HOME),
    };
    let config_string = match read_to_string(location) {
        Ok(config) => config,
        Err(error) => match error.kind() {
            ErrorKind::NotFound => {
                return Ok(None);
            }
            _ => Err(error)?,
        },
    };
    Ok(toml::from_str(&config_string)?)
}

pub fn get_config(config_file_location: Option<String>) -> Result<Config> {
    match read_config_file(config_file_location) {
        Ok(Some(config_from_file)) => Ok(Config::new().update(config_from_file)),
        Ok(None) => Ok(Config::new()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn test_read_config_file() -> Result<()> {
        println!(
            "{:?}",
            read_config_file(Some("./fixtures/config.toml".to_string()))
        );
        assert!(read_config_file(Some("./fixtures/config.toml".to_string()))?.is_some());
        Ok(())
    }

    #[test]
    fn test_config_file_missing() -> Result<()> {
        if let Ok(None) = read_config_file(Some("./fixtures/configx.toml".to_string())) {
            assert!(true);
        } else {
            assert!(false);
        }
        Ok(())
    }

    #[test]
    fn test_bad_config_file() -> Result<()> {
        if let Err(_) = read_config_file(Some("./fixtures/bad_config.toml".to_string())) {
            assert!(true);
        } else {
            assert!(false);
        }
        Ok(())
    }
}
