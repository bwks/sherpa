use serde_derive::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Domain {
    // #[serde(rename = "type")]
    // pub domain_type: String,
    // pub name: String,
    // pub memory: Memory,
    // pub vcpu: Vcpu,
    // pub resource: Resource,
    // pub os: Os,
    // pub features: Features,
    // pub cpu: Cpu,
    // pub clock: Clock,
    // pub on_poweroff: String,
    // pub on_reboot: String,
    // pub on_crash: String,
    // pub pm: PowerManagement,
    // pub devices: Devices,
    pub cpus: u8,
    pub memory: u16,
}
