UPDATE node_image
SET model = ?2, version = ?3, repo = ?4, os_variant = ?5, kind = ?6, bios = ?7, cpu_count = ?8,
    cpu_architecture = ?9, cpu_model = ?10, machine_type = ?11, vmx_enabled = ?12, memory = ?13,
    hdd_bus = ?14, cdrom = ?15, cdrom_bus = ?16, data_interface_count = ?17, interface_prefix = ?18,
    interface_type = ?19, interface_mtu = ?20, first_interface_index = ?21,
    management_interface = ?22, reserved_interface_count = ?23, "default" = ?24, boot_mode = ?25,
    ztp = ?26
WHERE id = ?1
RETURNING id, model, version, repo, os_variant, kind, bios, cpu_count, cpu_architecture, cpu_model,
    machine_type, vmx_enabled, memory, hdd_bus, cdrom, cdrom_bus,
    data_interface_count, interface_prefix, interface_type, interface_mtu, first_interface_index,
    management_interface, reserved_interface_count, "default", boot_mode, ztp
