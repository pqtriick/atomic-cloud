use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::process::exit;

use anyhow::Result;
use inquire::{required, Text};
use log::{error, warn};
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use validators::PortValidator;

use crate::config::auto_complete::SimpleAutoComplete;
use crate::config::validators::AddressValidator;

pub mod auto_complete;
pub mod validators;

pub const CONFIG_DIRECTORY: &str = "configs";
const CONFIG_FILE: &str = "config.toml";

#[derive(Deserialize, Serialize)]
pub struct Config {
    pub listener: Option<SocketAddr>,
}

impl Config {
    fn new_empty() -> Self {
        Self { listener: None }
    }

    fn load_or_empty() -> Self {
        let path = Path::new(CONFIG_DIRECTORY).join(CONFIG_FILE);
        if !path.exists() {
            return Self::new_empty();
        }
        Self::load_from_file(&path).unwrap_or_else(|error| {
            warn!("Failed to read configuration from file: {}", error);
            Self::new_empty()
        })
    }

    pub fn new_filled() -> Self {
        let mut config = Self::load_or_empty();

        if config.listener.is_none() {
            let address = Text::new("Which address should the TcpListener listen to?")
                .with_autocomplete(SimpleAutoComplete::from_slices(vec!["0.0.0.0", "127.0.0.1"]))
                .with_validator(AddressValidator::new())
                .with_validator(required!())
                .prompt()
                .unwrap_or_else(|error| {
                    error!("Failed to read address from user input: {}", error);
                    exit(1);
                });

            let port = Text::new("On which port should the TcpListener listen?")
                .with_autocomplete(SimpleAutoComplete::from_slices(vec!["51067"]))
                .with_validator(PortValidator::new())
                .with_validator(required!())
                .prompt()
                .unwrap_or_else(|error| {
                    error!("Failed to read port from user input: {}", error);
                    exit(1);
                });

            config.listener = Some(SocketAddr::new(address.parse().unwrap(), port.parse().unwrap()));
        }

        config.save_to_file(&Path::new(CONFIG_DIRECTORY).join(CONFIG_FILE)).unwrap_or_else(|error| {
            error!("Failed to save generated configuration to file: {}", error);
            exit(1);
        });
        config
    }
}

impl SaveToTomlFile for Config {}
impl LoadFromTomlFile for Config {}

pub trait SaveToTomlFile: Serialize {
    fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string(self)?)?;
        Ok(())
    }
}

pub trait LoadFromTomlFile: DeserializeOwned {
    fn load_from_file(path: &Path) -> Result<Self> {
        let data = fs::read_to_string(path)?;
        let config = toml::from_str(&data)?;
        Ok(config)
    }
}