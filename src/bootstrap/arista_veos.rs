use std::net::Ipv4Addr;

use askama::Template;

use crate::model::User;

pub fn arista_veos_ztp_script() -> String {
    r#"#!/usr/bin/env bash

# Define variables
USB_DEVICE="/dev/sdb"
MOUNT_POINT="/mnt/usb1"
CONFIG_FILE="startup-config"

# Function to mount USB
mount_usb() {
    # Create mount point if it doesn't exist
    if [ ! -d "$MOUNT_POINT" ]; then
        mkdir -p "$MOUNT_POINT"
    fi

    # Mount the USB device
    mount "$USB_DEVICE" "$MOUNT_POINT"
    if [ $? -ne 0 ]; then
        echo "Error: Failed to mount $USB_DEVICE at $MOUNT_POINT"
        exit 1
    fi

    echo "USB drive mounted successfully at $MOUNT_POINT"
}

# Function to copy configuration
copy_config() {
    # Check if the startup-config file exists on the USB drive
    if [ ! -f "$MOUNT_POINT/$CONFIG_FILE" ]; then
        echo "Error: $CONFIG_FILE not found on USB drive"
        exit 1
    fi

    # Copy startup-config to running-config using FastCli
    cp /mnt/usb1/startup-config /mnt/flash/startup-config
    FastCli -p 15 -c "zerotouch disable"

    if [ $? -ne 0 ]; then
        echo "Error: Failed to copy configuration from $CONFIG_FILE"
        exit 1
    fi

    echo "Configuration copied successfully from $CONFIG_FILE"
}

# Function to unmount USB
unmount_usb() {
    umount "$MOUNT_POINT"
    if [ $? -ne 0 ]; then
        echo "Warning: Failed to unmount USB drive from $MOUNT_POINT"
        exit 1
    fi

    echo "USB drive unmounted successfully from $MOUNT_POINT"
}

# Main script execution
mount_usb
copy_config
unmount_usb

exit 0
"#
    .to_owned()
}

#[derive(Template)]
#[template(
    source = r#"!
hostname {{ hostname }}
dns domain {{ crate::core::konst::SHERPA_DOMAIN_NAME }}
ip name-server {{ name_server }}
!
no aaa root
!
service routing protocols model multi-agent
!
aaa authorization exec default local
!
{%- for user in users %}
username {{ user.username }} privilege 15{% if let Some(password) = user.password %} secret {{ password }}{% endif %}
username {{ user.username }} ssh-key {{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}
{%- endfor %}
!
interface Management1
   ip address dhcp
!
management api http-commands
   no shutdown
!
end
!
"#,
    ext = "txt"
)]
pub struct AristaVeosZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub name_server: Ipv4Addr,
}
