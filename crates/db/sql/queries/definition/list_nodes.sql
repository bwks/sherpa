SELECT node.id, node.name, node_image.model, node.manifest_image, node.version,
    node.manifest_memory, node.manifest_cpu_count, node.cpu_mode, node.cpu_sockets,
    node.cpu_cores, node.cpu_threads, node.numa_node, node.numa_memory_mode,
    node.boot_disk_size, node.boot_disk_clone_method, node.boot_disk_cache, node.boot_disk_io,
    node.interfaces_data_count, node.interfaces_queues, node.ztp_method, node.ztp_config,
    node.manifest_ready_port, node.manifest_skip_ready_check, node.vm_watchdog,
    node.container_user, node.container_privileged, node.container_shm_size,
    node.container_platform, node.unikernel_cmdline
FROM node
JOIN node_image ON node_image.id = node.image
WHERE node.lab = ?1
ORDER BY node."index" ASC
