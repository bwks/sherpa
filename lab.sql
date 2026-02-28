-- ========================================= --
-- Sherpa Lab Database Schema
-- Updated to align with data::node::NodeVariant struct
-- ========================================= --

-- ========================================= --
-- User Schema
-- ========================================= --

DEFINE TABLE user SCHEMAFULL;
DEFINE FIELD username ON TABLE user TYPE string
    ASSERT string::len($value) >= 3
    AND $value = /^[a-zA-Z0-9@._-]+$/;
DEFINE FIELD ssh_keys ON TABLE user TYPE array<string> DEFAULT [];

DEFINE INDEX unique_username
  ON TABLE user FIELDS username UNIQUE;


-- ========================================= --
-- Node Config
-- ========================================= --

DEFINE TABLE node_config SCHEMAFULL;

DEFINE FIELD model ON TABLE node_config TYPE string
    ASSERT $value IN ["arista_veos", "arista_ceos", "aruba_aoscx", "cisco_asav", "cisco_csr1000v", "cisco_cat8000v", "cisco_cat9000v", "cisco_iosxrv9000", "cisco_nexus9300v", "cisco_iosv", "cisco_iosvl2", "cisco_ise", "cisco_ftdv", "juniper_vrouter", "juniper_vswitch", "juniper_vevolved", "juniper_vsrxv3", "nokia_srlinux", "alma_linux", "rocky_linux", "alpine_linux", "cumulus_linux", "centos_linux", "fedora_linux", "rhel_linux", "opensuse_linux", "suse_linux", "ubuntu_linux", "flatcar_linux", "sonic_linux", "windows_server", "free_bsd", "open_bsd", "surreal_db", "mysql_db", "postgresql_db", "generic_container", "generic_unikernel", "generic_vm"];
DEFINE FIELD version ON TABLE node_config TYPE string;
DEFINE FIELD repo ON TABLE node_config TYPE string DEFAULT "";

-- Operating System & Platform
DEFINE FIELD os_variant ON TABLE node_config TYPE string
    ASSERT $value IN ["eos", "aos", "asa", "ios", "iosxe", "iosxr", "ise", "nxos", "fxos", "junos", "bsd", "linux", "nvue", "sonic", "server2012", "srlinux", "unknown"];
DEFINE FIELD kind ON TABLE node_config TYPE string
    ASSERT $value IN ["container", "unikernel", "virtual_machine"];
DEFINE FIELD bios ON TABLE node_config TYPE string
    ASSERT $value IN ["sea_bios", "uefi"];

-- CPU Configuration
DEFINE FIELD cpu_count ON TABLE node_config TYPE number
    ASSERT $value >= 1 AND $value <= 255 AND $value == math::floor($value);
DEFINE FIELD cpu_architecture ON TABLE node_config TYPE string
    ASSERT $value IN ["x86_64"];
DEFINE FIELD cpu_model ON TABLE node_config TYPE string
    ASSERT $value IN ["host-model", "IvyBridge", "SandyBridge"];
DEFINE FIELD machine_type ON TABLE node_config TYPE string
    ASSERT $value IN ["pc", "q35", "pc-q35-5.0", "pc-q35-5.2", "pc-q35-6.0", "pc-q35-6.2", "pc-q35-8.0", "pc-q35-8.1", "pc-q35-8.2", "pc-i440fx-4.2", "pc-i440fx-5.1", "pc-i440fx-8.0", "pc-i440fx-8.1", "pc-i440fx-8.2"];
DEFINE FIELD vmx_enabled ON TABLE node_config TYPE bool;

-- Memory Configuration
DEFINE FIELD memory ON TABLE node_config TYPE number
    ASSERT $value >= 64 AND $value <= 65535 AND $value == math::floor($value);

-- Disk Configuration
DEFINE FIELD hdd_bus ON TABLE node_config TYPE string
    ASSERT $value IN ["ide", "sata", "scsi", "usb", "virtio"];
DEFINE FIELD cdrom ON TABLE node_config TYPE string DEFAULT "";
DEFINE FIELD cdrom_bus ON TABLE node_config TYPE string
    ASSERT $value IN ["ide", "sata", "scsi", "usb", "virtio"];

-- ZTP (Zero Touch Provisioning) Configuration
DEFINE FIELD ztp_enable ON TABLE node_config TYPE bool;
DEFINE FIELD ztp_method ON TABLE node_config TYPE string
    ASSERT $value IN ["cloud-init", "cdrom", "disk", "http", "ignition", "ipxe", "tftp", "usb", "volume", "none"];
DEFINE FIELD ztp_username ON TABLE node_config TYPE string DEFAULT "";
DEFINE FIELD ztp_password ON TABLE node_config TYPE string DEFAULT "";
DEFINE FIELD ztp_password_auth ON TABLE node_config TYPE bool;

-- Interface Configuration
DEFINE FIELD data_interface_count ON TABLE node_config TYPE number
    ASSERT $value >= 1 AND $value <= 255 AND $value == math::floor($value);
DEFINE FIELD interface_prefix ON TABLE node_config TYPE string;
DEFINE FIELD interface_type ON TABLE node_config TYPE string
    ASSERT $value IN ["e1000", "virtio", "vmxnet3", "host", "mac_vlan"];
DEFINE FIELD interface_mtu ON TABLE node_config TYPE number
    ASSERT $value >= 576 AND $value <= 9600 AND $value == math::floor($value);
DEFINE FIELD first_interface_index ON TABLE node_config TYPE number
    ASSERT $value >= 0 AND $value <= 255 AND $value == math::floor($value);
DEFINE FIELD dedicated_management_interface ON TABLE node_config TYPE bool;
DEFINE FIELD management_interface ON TABLE node_config TYPE string
    ASSERT $value IN ["eth0", "GigabitEthernet0/0", "GigabitEthernet1", "re0:mgmt-0", "fxp0", "fxp0.0", "mgmt", "mgmt0", "Management0/0", "Management1", "MgmtEth0/RP0/CPU0/0", "Vlan1"];
DEFINE FIELD reserved_interface_count ON TABLE node_config TYPE number
    ASSERT $value >= 0 AND $value <= 255 AND $value == math::floor($value);

-- Composite (name, kind) must be unique
DEFINE INDEX unique_node_config_name_kind
  ON TABLE node_config FIELDS model, kind UNIQUE;

-- ========================================= --
-- LAB Schema
-- ========================================= --

DEFINE TABLE lab SCHEMAFULL;
DEFINE FIELD lab_id ON TABLE lab TYPE string
    ASSERT string::len($value) >= 1;
DEFINE FIELD name ON TABLE lab TYPE string;
DEFINE FIELD user ON TABLE lab TYPE record<user>
    REFERENCE ON DELETE CASCADE;

-- lab_id must be unique
DEFINE INDEX unique_lab_id ON TABLE lab FIELDS lab_id UNIQUE;

-- Composite (name, user) must be unique
DEFINE INDEX unique_lab_name_user
  ON TABLE lab FIELDS name, user UNIQUE;

-- ========================================= --
-- Node Schema
-- ========================================= --

DEFINE TABLE node SCHEMAFULL;
DEFINE FIELD name ON TABLE node TYPE string;
DEFINE FIELD index ON TABLE node TYPE number
    -- Must be a u16 (0 to 65535) whole number
    ASSERT $value >= 0 AND $value <= 65535 AND $value == math::floor($value);
DEFINE FIELD config ON TABLE node TYPE record<node_config>
    REFERENCE ON DELETE REJECT;
DEFINE FIELD lab ON TABLE node TYPE record<lab>
    REFERENCE ON DELETE CASCADE;

-- Composite (lab, name) must be unique
DEFINE INDEX unique_node_name_per_lab
  ON TABLE node FIELDS lab, name UNIQUE;

-- Composite (lab, index) must be unique  
DEFINE INDEX unique_node_index_per_lab
  ON TABLE node FIELDS lab, index UNIQUE;

-- ========================================= --
-- Link Schema
-- ========================================= --

DEFINE TABLE link SCHEMAFULL;
DEFINE FIELD index ON TABLE link TYPE number
    ASSERT $value >= 0 AND $value <= 65535 AND $value == math::floor($value);
DEFINE FIELD node_a ON TABLE link TYPE record<node>
    REFERENCE ON DELETE CASCADE;
DEFINE FIELD node_b ON TABLE link TYPE record<node>
    REFERENCE ON DELETE CASCADE;
DEFINE FIELD int_a ON TABLE link TYPE string;
DEFINE FIELD int_b ON TABLE link TYPE string;
DEFINE FIELD bridge_a ON TABLE link TYPE string;
DEFINE FIELD bridge_b ON TABLE link TYPE string;
DEFINE FIELD veth_a ON TABLE link TYPE string;
DEFINE FIELD veth_b ON TABLE link TYPE string;
DEFINE FIELD kind ON TABLE link TYPE string
    ASSERT $value IN ["p2p_bridge", "p2p_udp", "p2p_veth"];
DEFINE FIELD lab ON TABLE link TYPE record<lab>
    REFERENCE ON DELETE CASCADE;

-- Composite (node_a, node_b, int_a, int_b) must be unique
DEFINE INDEX unique_peers_on_link
  ON TABLE link FIELDS node_a, node_b, int_a, int_b UNIQUE;


-- Users
INSERT INTO user [
    { username: "bradmin", ssh_keys: [] },
];

