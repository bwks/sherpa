-- One ZTP setting for a node image, `ztp`: `disabled`, or the one method Sherpa provisions the
-- image's nodes by. It replaces the `ztp_enable` flag and the `ztp_method` beside it, which
-- could disagree and could name a method the model has no generator for. Released migrations
-- never change, so the model and method names are written out.
ALTER TABLE node_image ADD COLUMN ztp TEXT NOT NULL DEFAULT 'disabled';

-- The first branch that matches decides.
UPDATE node_image SET ztp = CASE
    -- Switched off, whatever method was stored beside it.
    WHEN ztp_enable = 0 THEN 'disabled'
    -- The containers whose config Sherpa mounts. Their stored method was never read.
    WHEN model IN ('arista_ceos', 'nokia_srlinux', 'juniper_cevolved', 'frr_linux')
        THEN 'volume'
    -- The other containers and the unikernels: nothing generates a config for them.
    WHEN model IN ('gitlab_ce', 'forgejo_forge', 'surreal_db', 'mysql_db', 'mongo_db',
            'postgresql_db', 'nats_bus', 'hashicorp_vault', 'keycloak_server',
            'generic_container', 'generic_unikernel', 'unikraft_unikernel', 'nanos_unikernel')
        THEN 'disabled'
    -- A virtual machine with a method its model takes keeps it.
    WHEN ztp_method = 'cloud-init' AND model IN ('centos_linux', 'alma_linux', 'devbox_linux', 'rocky_linux', 'fedora_linux',
            'opensuse_linux', 'redhat_linux', 'suse_linux', 'ubuntu_linux', 'omarchy_linux',
            'nixos_linux', 'kali_linux', 'infrahub_server', 'jenkins_server', 'nautobot_server',
            'netbox_server', 'signoz_server', 'virt_server', 'generic_vm', 'free_bsd', 'open_bsd',
            'devbox_windows', 'windows_server', 'alpine_linux')
        THEN 'cloud-init'
    WHEN ztp_method = 'tftp'
        AND model IN ('arista_veos', 'aruba_aoscx', 'juniper_vevolved', 'juniper_vsrxv3')
        THEN 'tftp'
    WHEN ztp_method = 'cdrom' AND model IN ('cisco_csr1000v', 'cisco_cat8000v', 'cisco_cat9000v', 'cisco_asav',
            'cisco_nexus9300v', 'cisco_iosxrv9000', 'cisco_ftdv', 'juniper_vsrxv3',
            'juniper_vrouter', 'juniper_vswitch', 'paloalto_panos')
        THEN 'cdrom'
    WHEN ztp_method = 'disk' AND model IN ('cisco_iosv', 'cisco_iosvl2', 'cisco_ise', 'mikrotik_chr')
        THEN 'disk'
    WHEN ztp_method = 'usb' AND model IN ('cumulus_linux', 'juniper_vevolved') THEN 'usb'
    WHEN ztp_method = 'http' AND model = 'sonic_linux' THEN 'http'
    WHEN ztp_method = 'ignition' AND model = 'flatcar_linux' THEN 'ignition'
    WHEN ztp_method = 'nocloud' AND model = 'talos_linux' THEN 'nocloud'
    -- Methods that generated nothing for a virtual machine.
    WHEN ztp_method IN ('none', 'volume', 'ipxe') THEN 'disabled'
    -- A virtual machine with a method its model does not take, on which `up` failed: the
    -- model's own method.
    WHEN model IN ('centos_linux', 'alma_linux', 'devbox_linux', 'rocky_linux', 'fedora_linux',
            'opensuse_linux', 'redhat_linux', 'suse_linux', 'ubuntu_linux', 'omarchy_linux',
            'nixos_linux', 'kali_linux', 'infrahub_server', 'jenkins_server', 'nautobot_server',
            'netbox_server', 'signoz_server', 'virt_server', 'generic_vm', 'free_bsd', 'open_bsd',
            'devbox_windows', 'windows_server', 'alpine_linux')
        THEN 'cloud-init'
    WHEN model IN ('arista_veos', 'aruba_aoscx', 'juniper_vevolved') THEN 'tftp'
    WHEN model IN ('cisco_csr1000v', 'cisco_cat8000v', 'cisco_cat9000v', 'cisco_asav',
            'cisco_nexus9300v', 'cisco_iosxrv9000', 'cisco_ftdv', 'juniper_vsrxv3',
            'juniper_vrouter', 'juniper_vswitch', 'paloalto_panos')
        THEN 'cdrom'
    WHEN model IN ('cisco_iosv', 'cisco_iosvl2', 'cisco_ise', 'mikrotik_chr') THEN 'disk'
    WHEN model = 'cumulus_linux' THEN 'usb'
    WHEN model = 'sonic_linux' THEN 'http'
    WHEN model = 'flatcar_linux' THEN 'ignition'
    WHEN model = 'talos_linux' THEN 'nocloud'
    -- A model this release does not know.
    ELSE 'disabled'
END;

ALTER TABLE node_image DROP COLUMN ztp_enable;
ALTER TABLE node_image DROP COLUMN ztp_method;
