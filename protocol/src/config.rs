use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DEFAULT_PORT: u16 = 62115;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_linux_root_drive")]
    pub linux_root_drive: String,
    #[serde(default = "default_hgfs_prefix")]
    pub hgfs_prefix: String,
}

fn default_host() -> String {
    // Default: the VMware host gateway is typically at .1 on the NAT subnet.
    // Users should override this in the config file.
    "172.16.0.1".to_string()
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn default_linux_root_drive() -> String {
    "W".to_string()
}

fn default_hgfs_prefix() -> String {
    "/mnt/hgfs/".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            linux_root_drive: default_linux_root_drive(),
            hgfs_prefix: default_hgfs_prefix(),
        }
    }
}

impl Config {
    /// Load config from the platform-appropriate path.
    pub fn load() -> Self {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}

/// Platform-appropriate config file path.
fn config_path() -> PathBuf {
    if cfg!(windows) {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("interop")
            .join("interop.toml")
    } else {
        PathBuf::from("/etc/interop.toml")
    }
}
