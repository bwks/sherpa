INSERT INTO node_image (id, model, version, kind, cpu_count, memory, data_interface_count, management_interface,
    reserved_interface_count, "default", os_variant, cpu_architecture, cpu_model, vmx_enabled,
    machine_type, bios, hdd_bus, cdrom, cdrom_bus, interface_type, interface_mtu, ztp, repo,
    boot_mode)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
    ?21, ?22, ?23, ?24)
RETURNING id, model, version, kind, cpu_count, memory, data_interface_count, management_interface,
    reserved_interface_count, "default", os_variant, cpu_architecture, cpu_model, vmx_enabled,
    machine_type, bios, hdd_bus, cdrom, cdrom_bus, interface_type, interface_mtu, ztp, repo,
    boot_mode
