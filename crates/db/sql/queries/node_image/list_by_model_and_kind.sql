SELECT id, model, version, kind, cpu_count, memory, data_interface_count, management_interface,
    reserved_interface_count, "default", os_variant, cpu_architecture, cpu_model, vmx_enabled,
    machine_type, bios, hdd_bus, cdrom, cdrom_bus, interface_type, interface_mtu, ztp, repo,
    boot_mode
FROM node_image
WHERE model = ?1 AND kind = ?2
ORDER BY version ASC
