UPDATE node
SET manifest_image = ?2, version = ?3, manifest_memory = ?4, manifest_cpu_count = ?5,
    cpu_mode = ?6, cpu_sockets = ?7, cpu_cores = ?8, cpu_threads = ?9, numa_node = ?10,
    numa_memory_mode = ?11, boot_disk_size = ?12, boot_disk_clone_method = ?13,
    boot_disk_cache = ?14, boot_disk_io = ?15, interfaces_data_count = ?16,
    interfaces_queues = ?17, ztp_method = ?18, ztp_config = ?19, manifest_ready_port = ?20,
    manifest_skip_ready_check = ?21, vm_watchdog = ?22, container_user = ?23,
    container_privileged = ?24, container_shm_size = ?25, container_platform = ?26,
    unikernel_cmdline = ?27
WHERE id = ?1
