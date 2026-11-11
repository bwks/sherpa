INSERT INTO node_image (id, model, version, repo, os_variant, kind, bios, cpu_count, cpu_architecture, cpu_model,
    machine_type, vmx_enabled, memory, hdd_bus, cdrom, cdrom_bus, ztp_username, ztp_password,
    ztp_password_auth, data_interface_count, interface_prefix, interface_type, interface_mtu,
    first_interface_index, management_interface, reserved_interface_count, "default", boot_mode,
    ztp)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
    ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)
RETURNING id, model, version, repo, os_variant, kind, bios, cpu_count, cpu_architecture, cpu_model,
    machine_type, vmx_enabled, memory, hdd_bus, cdrom, cdrom_bus, ztp_username, ztp_password,
    ztp_password_auth, data_interface_count, interface_prefix, interface_type, interface_mtu,
    first_interface_index, management_interface, reserved_interface_count, "default", boot_mode,
    ztp
