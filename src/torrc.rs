use anyhow::{anyhow, Result};
use lazy_static::lazy_static;
use regex::Regex;
use std::fs::File;
use std::io::{BufRead, BufReader, Lines};
use std::path::PathBuf;
use std::str::FromStr;
use tor_client_lib::control_connection::{OnionServiceMapping, TorSocketAddr};

lazy_static! {
    static ref TORRC_PATH: PathBuf = PathBuf::from("/etc/tor/torrc");
    static ref DATA_DIR_RE: Regex = Regex::new(r"^DataDirectory (?P<data_dir>.*)$").unwrap();
    static ref ONION_SERVICE_DIR: Regex = Regex::new(r"^HiddenServiceDir (?P<dir>.*)$").unwrap();
    static ref ONION_SERVICE_PORT: Regex =
        Regex::new(r"^HiddenServicePort (?P<virt_port>[^ ]*) (?P<target>.*)$").unwrap();
}

#[derive(Clone, Debug)]
pub struct OnionServiceInfo {
    name: String,
    dir: String,
    ports: Vec<OnionServiceMapping>,
}

impl OnionServiceInfo {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn dir(&self) -> &str {
        &self.dir
    }

    pub fn ports(&self) -> &Vec<OnionServiceMapping> {
        &self.ports
    }
}

struct TorrcParser {
    lines: Lines<BufReader<File>>,
    current_line: Option<String>,
    reuse_line: bool,
}

impl TorrcParser {
    fn new() -> Result<Self> {
        let file = File::open(&*TORRC_PATH)?;
        Ok(Self {
            lines: BufReader::new(file).lines(),
            current_line: None,
            reuse_line: false,
        })
    }

    fn get_line(&mut self) -> Result<Option<String>> {
        if self.reuse_line {
            self.reuse_line = false;
            Ok(self.current_line.clone())
        } else {
            match self.lines.next() {
                Some(Ok(line)) => {
                    self.current_line = Some(line.clone());
                    Ok(Some(line.clone()))
                }
                Some(Err(error)) => Err(error.into()),
                None => Ok(None),
            }
        }
    }

    fn parse_onion_service(
        &mut self,
        data_dir: &str,
        onion_service_dir: &str,
    ) -> Result<OnionServiceInfo> {
        let re = Regex::new(&format!("^{}/(?P<name>[^/]*)/?", data_dir)).unwrap();
        let name = match re.captures(onion_service_dir) {
            Some(captures) => captures["name"].to_string(),
            None => {
                return Err(anyhow!("Error parsing 'HiddenServiceDir' line from torrc"));
            }
        };
        let mut ports = Vec::new();
        while let Some(line) = self.get_line()? {
            if line.starts_with('#') || line.is_empty() {
                continue;
            } else if let Some(captures) = ONION_SERVICE_PORT.captures(&line) {
                let virt_port = captures["virt_port"].parse::<u16>()?;
                let target = TorSocketAddr::from_str(&captures["target"])?;
                ports.push(OnionServiceMapping::new(virt_port, Some(target)));
            } else {
                self.reuse_line = true;
                break;
            }
        }
        Ok(OnionServiceInfo {
            name,
            dir: onion_service_dir.to_string(),
            ports,
        })
    }

    fn parse(&mut self) -> Result<Vec<OnionServiceInfo>> {
        let mut ret = Vec::new();
        let mut data_dir = "/var/lib/tor".to_string();
        while let Some(line) = self.get_line()? {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some(captures) = DATA_DIR_RE.captures(&line) {
                data_dir = String::from_str(&captures["data_dir"]).unwrap();
            } else if let Some(captures) = ONION_SERVICE_DIR.captures(&line) {
                ret.push(self.parse_onion_service(&data_dir, &captures["dir"])?);
            }
        }
        Ok(ret)
    }
}

pub fn get_onion_services() -> Result<Vec<OnionServiceInfo>> {
    TorrcParser::new()?.parse()
}

pub fn find_torrc_onion_service(name: &str) -> Result<Option<OnionServiceInfo>> {
    Ok(get_onion_services()?
        .iter()
        .find(|s| s.name == name)
        .cloned())
}
