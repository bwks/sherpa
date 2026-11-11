UPDATE node_image
SET model = ?2, version = ?3, repo = ?4, os_variant = ?5, kind = ?6, bios = ?7, cpu_count = ?8,
    cpu_architecture = ?9, cpu_model = ?10, machine_type = ?11, vmx_enabled = ?12, memory = ?13,
    hdd_bus = ?14, cdrom = ?15, cdrom_bus = ?16, ztp_username = ?17, ztp_password = ?18,
    ztp_password_auth = ?19, data_interface_count = ?20, interface_prefix = ?21, interface_type =
    ?22, interface_mtu = ?23, first_interface_index = ?24, management_interface = ?25,
    reserved_interface_count = ?26, "default" = ?27, boot_mode = ?28, ztp = ?29
WHERE id = ?1
RETURNING id, model, version, repo, os_variant, kind, bios, cpu_count, cpu_architecture, cpu_model,
    machine_type, vmx_enabled, memory, hdd_bus, cdrom, cdrom_bus, ztp_username, ztp_password,
    ztp_password_auth, data_interface_count, interface_prefix, interface_type, interface_mtu,
    first_interface_index, management_interface, reserved_interface_count, "default", boot_mode,
    ztp
