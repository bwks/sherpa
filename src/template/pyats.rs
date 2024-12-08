use std::collections::HashMap;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use crate::core::Config;
use crate::topology::Manifest;

#[derive(Debug, Serialize, Deserialize)]
pub struct Credentials {
    pub password: String,
    pub username: String,
    pub ssh_config: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Connection {
    pub ip: String,
    pub protocol: String,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Device {
    pub alias: String,
    pub connections: HashMap<String, Connection>,
    pub credentials: HashMap<String, Credentials>,
    pub os: String,
    pub platform: String,
    #[serde(rename = "type")]
    pub device_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PyatsInventory {
    pub devices: HashMap<String, Device>,
}

impl PyatsInventory {
    /*
      devices:
    iosxr1:
      # Step 1: OS and Type
      type: iosxr-Prod
      os: iosxr
      # Step 2: credentials
      credentials:
        default:
          username: admin
          password: Hacker@204k
      # Step 3: connection parameters
      connections:
        vty:
          protocol: ssh
          ip: test-mgmt.talesoftechnology.com
          port: 8080
      */
    pub fn from_manifest(manifest: &Manifest, config: &Config) -> Result<PyatsInventory> {
        // https://devnet-pubhub-site.s3.amazonaws.com/media/pyats/docs/topology/schema.html#schema
        let mut devices = HashMap::new();
        for device in &manifest.devices {
            let device_model = config
                .device_models
                .iter()
                .find(|d| d.name == device.device_model)
                .ok_or_else(|| {
                    anyhow::anyhow!("Device model not found: {}", device.device_model)
                })?;

            // Create connections map
            let mut connections = HashMap::new();
            connections.insert(
                "mgmt".to_string(),
                Connection {
                    ip: device.name.to_owned(),
                    protocol: "ssh".to_owned(),
                    port: 22,
                },
            );

            // Create credentials map
            let mut credentials = HashMap::new();
            credentials.insert(
                "default".to_string(),
                Credentials {
                    username: config.ztp_server.username.to_owned(),
                    password: config.ztp_server.password.to_owned(),
                    ssh_config: ".tmp/sherpa_ssh_config".to_owned(),
                },
            );

            // Create device entry
            let device_entry = Device {
                alias: device.name.clone(),
                connections,
                credentials,
                os: device_model.os_variant.to_string(),
                platform: device_model.os_variant.to_string(),
                device_type: device_model.name.to_string(),
            };

            devices.insert(device.name.clone(), device_entry);
        }
        Ok(PyatsInventory { devices })
    }
    pub fn to_yaml(&self) -> Result<String> {
        let yaml = serde_yaml::to_string(&self)?;
        Ok(yaml)
    }
}
