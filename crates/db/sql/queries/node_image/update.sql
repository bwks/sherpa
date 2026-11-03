UPDATE node_image
SET model = ?2, version = ?3, repo = ?4, os_variant = ?5, kind = ?6, bios = ?7, cpu_count = ?8,
    cpu_architecture = ?9, cpu_model = ?10, machine_type = ?11, vmx_enabled = ?12, memory = ?13,
    hdd_bus = ?14, cdrom = ?15, cdrom_bus = ?16, ztp_enable = ?17, ztp_method = ?18,
    ztp_username = ?19, ztp_password = ?20, ztp_password_auth = ?21, data_interface_count = ?22,
    interface_prefix = ?23, interface_type = ?24, interface_mtu = ?25, first_interface_index = ?26,
    dedicated_management_interface = ?27, management_interface = ?28,
    reserved_interface_count = ?29, "default" = ?30, boot_mode = ?31
WHERE id = ?1
RETURNING id, model, version, repo, os_variant, kind, bios, cpu_count, cpu_architecture, cpu_model,
    machine_type, vmx_enabled, memory, hdd_bus, cdrom, cdrom_bus, ztp_enable, ztp_method,
    ztp_username, ztp_password, ztp_password_auth, data_interface_count, interface_prefix,
    interface_type, interface_mtu, first_interface_index, dedicated_management_interface,
    management_interface, reserved_interface_count, "default", boot_mode
