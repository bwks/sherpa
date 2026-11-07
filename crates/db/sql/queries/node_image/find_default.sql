SELECT id, model, version, repo, os_variant, kind, bios, cpu_count, cpu_architecture, cpu_model,
    machine_type, vmx_enabled, memory, hdd_bus, cdrom, cdrom_bus, ztp_enable, ztp_method,
    ztp_username, ztp_password, ztp_password_auth, data_interface_count, interface_prefix,
    interface_type, interface_mtu, first_interface_index, management_interface, reserved_interface_count, "default", boot_mode
FROM node_image
WHERE model = ?1 AND kind = ?2 AND "default" = 1
