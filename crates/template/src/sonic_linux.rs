use std::net::Ipv4Addr;

use askama::Template;
use serde_json::json;

use data::User;
use konst::{DEVICE_CONFIGS_DIR, HTTP_PORT};

#[derive(Template)]
#[template(path = "sonic/ztp_user.jinja", ext = "txt")]
pub struct SonicLinuxUserTemplate {
    pub user: User,
}

pub struct SonicLinuxZtp {}

impl SonicLinuxZtp {
    pub fn file_map(device_name: &str, ztp_server: &Ipv4Addr) -> String {
        let sonic_ztp_template = json!(
            {
                "ztp": {
                  "001-configdb-json": {
                    "url": {
                      "source": format!("http://{ztp_server}:{HTTP_PORT}/{DEVICE_CONFIGS_DIR}/{device_name}_config_db.json"),
                      "destination": "/etc/sonic/config_db.json",
                      "secure": false
                    }
                  },
                  "002-set-password": {
                      "plugin": {
                        "url": format!("http://{ztp_server}:{HTTP_PORT}/{DEVICE_CONFIGS_DIR}/sonic_ztp_user.sh"),
                        "shell": "true"
                       },
                       "reboot-on-success": false
                    }
                }
            }
        );
        sonic_ztp_template.to_string()
    }
    pub fn config(device_name: &str) -> String {
        let template = json!({
          "DEVICE_METADATA": {
            "localhost": {
              "hostname": device_name
            }
          },
          "AAA": {
            "authentication": {
              "login": "local"
            }
          }
        });
        template.to_string()
    }
}
