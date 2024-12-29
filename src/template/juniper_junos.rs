// use std::net::Ipv4Addr;

use rinja::Template;

use crate::core::konst::JUNIPER_ZTP_CONFIG;
use crate::data::User;

pub fn juniper_vevolved_ztp_script() -> String {
    format!(
        r#"#!/bin/sh

# Define the path to the configuration file
CONFIG_FILE="/tmp/vmmusb/{JUNIPER_ZTP_CONFIG}"

# Check if the configuration file exists
if [ ! -f "$CONFIG_FILE" ]; then
    echo "Error: Configuration file not found at $CONFIG_FILE"
    exit 1
fi

# Load the configuration
cli -c "configure; load override $CONFIG_FILE; commit and-quit"

# Check the exit status
if [ $? -eq 0 ]; then
    echo "Configuration loaded and committed successfully"
else
    echo "Error: Failed to load or commit the configuration"
    exit 1
fi
"#
    )
}

#[derive(Template)]
#[template(path = "juniper/juniper_junos.jinja", ext = "txt")]
pub struct JunipervJunosZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub mgmt_interface: String,
}
