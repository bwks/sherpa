use std::net::{Ipv4Addr, Ipv6Addr};

use askama::Template;
use serde_json::json;

use shared::data::{NetworkV4, NetworkV6, OwnerUser, User};
use shared::konst::{HTTP_PORT, NODE_CONFIGS_DIR};

#[derive(Template)]
#[template(path = "sonic/ztp_user.jinja", ext = "txt")]
pub struct SonicLinuxUserTemplate {
    pub user: User,
    pub owner: Option<OwnerUser>,
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
    pub fn user_script_filename(device_name: &str) -> String {
        format!("{device_name}_ztp_user.sh")
    }

    pub fn file_map(device_name: &str, ztp_server: &Ipv4Addr) -> String {
        let user_script = Self::user_script_filename(device_name);
        let sonic_ztp_template = json!(
            {
                "ztp": {
                  "001-configdb-json": {
                    "url": {
                      "source": format!("http://{ztp_server}:{HTTP_PORT}/{NODE_CONFIGS_DIR}/{device_name}_config_db.json"),
                      "destination": "/etc/sonic/config_db.json",
                      "secure": false
                    }
                  },
                  "002-set-password": {
                      "plugin": {
                        "url": format!("http://{ztp_server}:{HTTP_PORT}/{NODE_CONFIGS_DIR}/{user_script}"),
                        "shell": "true"
                       },
                       "reboot-on-success": false
                    }
                }
            }
        );
        sonic_ztp_template.to_string()
    }
    pub fn config(&self) -> String {
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
          }
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
        }

        template.to_string()
    }
}
