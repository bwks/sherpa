use askama::Template;

use data::{Dns, User};
use konst::{ARUBA_ZTP_SCRIPT, SHERPA_PASSWORD};

#[allow(dead_code)]
pub fn aruba_aoscx_ztp_script() -> String {
    format!(
        r#"!
start-shell
sudo mkdir /mnt/config/
sudo mount /dev/sdb /mnt/config/
/bin/sh /mnt/config/{ARUBA_ZTP_SCRIPT}
    "#,
    )
}

#[derive(Template)]
#[template(path = "aruba/aruba_aoscx.jinja", ext = "txt")]
pub struct ArubaAoscxTemplate {
    pub hostname: String,
    pub user: User,
    pub dns: Dns,
}

#[derive(Template)]
#[template(path = "aruba/aruba_aoscx_sh.jinja", ext = "txt")]
pub struct ArubaAoscxShTemplate {
    pub hostname: String,
    pub user: User,
    pub dns: Dns,
}
