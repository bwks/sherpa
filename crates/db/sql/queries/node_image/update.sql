UPDATE node_image
SET model = ?2, version = ?3, kind = ?4, cpu_count = ?5, memory = ?6, data_interface_count = ?7,
    management_interface = ?8, reserved_interface_count = ?9, "default" = ?10, os_variant = ?11,
    cpu_architecture = ?12, cpu_model = ?13, vmx_enabled = ?14, machine_type = ?15, bios = ?16,
    hdd_bus = ?17, cdrom = ?18, cdrom_bus = ?19, interface_type = ?20, interface_mtu = ?21,
    ztp = ?22, repo = ?23, boot_mode = ?24
WHERE id = ?1
RETURNING id, model, version, kind, cpu_count, memory, data_interface_count, management_interface,
    reserved_interface_count, "default", os_variant, cpu_architecture, cpu_model, vmx_enabled,
    machine_type, bios, hdd_bus, cdrom, cdrom_bus, interface_type, interface_mtu, ztp, repo,
    boot_mode
