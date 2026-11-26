use std::net::{Ipv4Addr, Ipv6Addr};

use askama::Template;
use serde_json::{Map, Value, json};

use shared::data::{NetworkV4, NetworkV6, OwnerUser};
use shared::konst::{HTTP_PORT, SONIC_PORT_COUNT, SONIC_PORT_NAME_STEP};

#[derive(Template)]
#[template(path = "sonic/ztp_user.jinja", ext = "txt")]
pub struct SonicLinuxUserTemplate {
    pub owner: OwnerUser,
}

/// The `supervisord` config a `sonic_linux` container starts with: its only program is
/// `init_script`, Sherpa's start script, which hands over to the image's own `supervisord`.
#[derive(Template)]
#[template(path = "sonic/container_supervisord.jinja", ext = "txt")]
pub struct SonicLinuxContainerSupervisordTemplate {
    pub init_script: String,
}

/// Sherpa's start script for a `sonic_linux` container: the owner's account when there is one, a wait
/// for the `data_count` data interfaces, the config at `config_db` when there is one, then the
/// SSH server, last, as the readiness signal.
#[derive(Template)]
#[template(path = "sonic/container_init.jinja", ext = "txt")]
pub struct SonicLinuxContainerInitTemplate {
    pub data_count: u8,
    pub owner: Option<OwnerUser>,
    pub config_db: Option<String>,
    pub mgmt: SonicLinuxContainerManagement,
}

/// The management addresses a `sonic_linux` container sets on `eth0` at each start, with their
/// gateways.
pub struct SonicLinuxContainerManagement {
    pub ipv4_address: Ipv4Addr,
    pub ipv4_prefix_length: u8,
    pub ipv4_gateway: Ipv4Addr,
    pub ipv6: Option<SonicLinuxContainerManagementV6>,
}

pub struct SonicLinuxContainerManagementV6 {
    pub address: Ipv6Addr,
    pub prefix_length: u8,
    pub gateway: Ipv6Addr,
}

pub struct SonicLinuxZtp {
    pub hostname: String,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
}

impl SonicLinuxZtp {
    /// The node's own account script, so nodes in one lab never overwrite each other's.
    #[must_use]
    pub fn user_script_filename(device_name: &str) -> String {
        format!("{device_name}_ztp_user.sh")
    }

    /// The ZTP JSON: load the node's `config_db.json`, run its account script, then load the
    /// `config_db.json` again to bring the linked ports back up.
    ///
    /// The first `configdb-json` step sets two options away from the plugin's defaults:
    /// - `clear-config: false` loads the file on top of the configuration the switch generated at
    ///   first boot (`config load`) instead of replacing it (`config reload`). The file holds only
    ///   what Sherpa sets, and the generated configuration holds the `PORT` table, so a reload
    ///   left the switch with no front-panel ports.
    /// - `save-config: true` then writes the merged configuration back to
    ///   `/etc/sonic/config_db.json` (`config save`). The download replaced that file with
    ///   Sherpa's partial one, which the switch loads on every boot after, so without the save the
    ///   ports were gone again after a reboot.
    ///
    /// Either option makes the plugin stop ZTP's in-band discovery by restarting the network
    /// configuration, which takes down the kernel interface of every port linked at boot while
    /// the configuration still has it up. So the last step loads the same file again, neither
    /// clearing nor saving and with no destination: discovery is over, so nothing restarts,
    /// the load sets `admin_status` again and the switch brings those interfaces back up. With
    /// no destination the plugin deletes the download, so the saved configuration stays whole.
    #[must_use]
    pub fn file_map(device_name: &str, ztp_server: &Ipv4Addr) -> String {
        let user_script = Self::user_script_filename(device_name);
        let config_url = format!("http://{ztp_server}:{HTTP_PORT}/{device_name}_config_db.json");
        let sonic_ztp_template = json!(
            {
                "ztp": {
                  "001-configdb-json": {
                    "url": {
                      "source": config_url,
                      "destination": "/etc/sonic/config_db.json",
                      "secure": false
                    },
                    "clear-config": false,
                    "save-config": true
                  },
                  "002-set-password": {
                      "plugin": {
                        "url": format!("http://{ztp_server}:{HTTP_PORT}/{user_script}"),
                        "shell": "true"
                       },
                       "reboot-on-success": false
                    },
                  "003-configdb-json": {
                    "url": {
                      "source": config_url,
                      "secure": false
                    },
                    "clear-config": false,
                    "save-config": false
                  }
                }
            }
        );
        sonic_ztp_template.to_string()
    }
    /// The node's `config_db.json`, which ZTP loads on top of the generated configuration: the
    /// hostname, local login, every front-panel port admin up, and the management addresses when
    /// the node has static ones.
    #[must_use]
    pub fn config(&self) -> String {
        let ports: Map<String, Value> = (0..u16::from(SONIC_PORT_COUNT))
            .map(|index| {
                (
                    format!("Ethernet{}", index * SONIC_PORT_NAME_STEP),
                    json!({ "admin_status": "up" }),
                )
            })
            .collect();
        let mut template = json!({
          "DEVICE_METADATA": {
            "localhost": {
              "hostname": self.hostname
            }
          },
          "AAA": {
            "authentication": {
              "login": "local"
            }
          },
          "PORT": ports
        });

        // Add MGMT_PORT and MGMT_INTERFACE only if mgmt_ip is provided
        if let Some(mgmt_ip) = self.mgmt_ipv4_address {
            template["MGMT_PORT"] = json!({
                "eth0": {
                    "alias": "eth0",
                    "admin_status": "up"
                }
            });

            template["MGMT_INTERFACE"] = json!({
                format!("eth0|{}/{}", mgmt_ip, self.mgmt_ipv4.prefix_length): {
                    "gwaddr": format!("{}", self.mgmt_ipv4.first)
                }
            });

            if let (Some(mgmt_ipv6_address), Some(mgmt_ipv6)) =
                (self.mgmt_ipv6_address, &self.mgmt_ipv6)
            {
                template["MGMT_INTERFACE"]
                    [format!("eth0|{mgmt_ipv6_address}/{}", mgmt_ipv6.prefix_length)] =
                    json!({ "gwaddr": mgmt_ipv6.first.to_string() });
            }
        }

        template.to_string()
    }
}
