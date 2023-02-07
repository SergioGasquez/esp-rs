use crate::{
    emoji,
    error::Error,
    host_triple::HostTriple,
    targets::Target,
    toolchain::{llvm::Llvm, rust::XtensaRust},
};
use directories::ProjectDirs;
use log::info;
use miette::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{create_dir_all, read, remove_file, write},
    path::PathBuf,
};

/// Deserialized contents of a configuration file
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct Config {
    /// Destination of the generated export file.
    pub export_file: Option<PathBuf>,
    /// Host triple
    pub host_triple: HostTriple,
    /// LLVM toolchain path.
    pub llvm: Option<Llvm>,
    /// Nightly Rust toolchain version.
    pub nightly_version: Option<String>,
    /// List of targets instaled.
    pub targets: HashSet<Target>,
    /// Xtensa Rust toolchain.
    pub xtensa_rust: Option<XtensaRust>,
}

pub struct ConfigFile {
    pub path: PathBuf,
    pub config: Config,
}

impl ConfigFile {
    /// Construcs a new config file with the given path and config
    pub fn new(config_path: &Option<PathBuf>, config: Config) -> Result<Self, Error> {
        let config_path = config_path.clone().unwrap_or(Self::get_config_path()?);
        Ok(ConfigFile {
            path: config_path,
            config,
        })
    }

    /// Load the config from config file
    pub fn load(config_path: &Option<PathBuf>) -> Result<Self, Error> {
        let config_path = config_path.clone().unwrap_or(Self::get_config_path()?);
        let config: Config = if let Ok(data) = read(&config_path) {
            toml::from_str(std::str::from_utf8(&data).unwrap())
                .map_err(|_| Error::FailedToDeserialize)?
        } else {
            return Err(Error::FileNotFound(
                config_path.to_string_lossy().into_owned(),
            ));
        };
        Self::new(&Some(config_path), config)
    }

    /// Save the config to file
    pub fn save(&self) -> Result<(), Error> {
        let serialized =
            toml::to_string(&self.config.clone()).map_err(|_| Error::FailedToSerialize)?;
        create_dir_all(self.path.parent().unwrap()).map_err(|_| Error::FailedToCreateConfigFile)?;
        write(&self.path, serialized)
            .map_err(|_| Error::FailedToWrite(self.path.display().to_string()))?;
        Ok(())
    }

    /// Delete the config file
    pub fn delete(&self) -> Result<(), Error> {
        info!("{} Deleting config file", emoji::WRENCH);
        remove_file(&self.path)
            .map_err(|_| Error::FailedToRemoveFile(self.path.display().to_string()))?;
        Ok(())
    }

    /// Gets the default path to the configuration file.
    pub fn get_config_path() -> Result<PathBuf, Error> {
        let dirs = ProjectDirs::from("rs", "esp", "espup").unwrap();
        let file = dirs.config_dir().join("espup.toml");
        Ok(file)
    }
}
