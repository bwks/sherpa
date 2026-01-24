use std::net::Ipv4Addr;

use askama::Template;
use serde_json::json;

use data::{NetworkV4, User};
use konst::{NODE_CONFIGS_DIR, HTTP_PORT};

#[derive(Template)]
#[template(path = "sonic/ztp_user.jinja", ext = "txt")]
pub struct SonicLinuxUserTemplate {
    pub user: User,
}

pub struct SonicLinuxZtp {
    pub hostname: String,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
}

impl SonicLinuxZtp {
    pub fn file_map(device_name: &str, ztp_server: &Ipv4Addr) -> String {
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
                        "url": format!("http://{ztp_server}:{HTTP_PORT}/{NODE_CONFIGS_DIR}/sonic_ztp_user.sh"),
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
