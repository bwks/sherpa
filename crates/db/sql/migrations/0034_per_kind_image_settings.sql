-- A node image holds the settings every kind takes and only its own kind's (#613, #614). The
-- columns of one kind become nullable, and a CHECK ties each to the kinds that take it: set for
-- those kinds and NULL for every other. A container image loses the CPU, machine, disk, CD-ROM,
-- interface device and OS settings no container code read; a unikernel loses the firmware,
-- disks, CD-ROM, vmx, OS and ZTP settings; every image loses `interface_prefix` and
-- `first_interface_index`, which nothing read.
--
-- `node.image` references `node_image`, so the table cannot be rebuilt under its foreign key.
-- Each column is added again instead, as `<name>_kind`, filled from the old one in one UPDATE,
-- since every CHECK is tested on each row an UPDATE writes, then the old column is dropped and
-- the new one takes its name. Each CHECK is written with AND and OR: Turso cannot read back a
-- comparison of two conditions, such as `(kind IN (...)) = (x IS NOT NULL)`, when it re-reads
-- the schema of a STRICT table, as `VACUUM INTO` does. A unikernel stored without a boot mode
-- boots its kernel directly, as the code read it. Released migrations never change, so the kind
-- and value names are written out.
ALTER TABLE node_image ADD COLUMN os_variant_kind TEXT
    CHECK ((kind = 'virtual_machine' AND os_variant_kind IS NOT NULL)
        OR (kind != 'virtual_machine' AND os_variant_kind IS NULL));
ALTER TABLE node_image ADD COLUMN cpu_architecture_kind TEXT
    CHECK ((kind IN ('virtual_machine', 'unikernel') AND cpu_architecture_kind IS NOT NULL)
        OR (kind NOT IN ('virtual_machine', 'unikernel') AND cpu_architecture_kind IS NULL));
ALTER TABLE node_image ADD COLUMN cpu_model_kind TEXT
    CHECK ((kind IN ('virtual_machine', 'unikernel') AND cpu_model_kind IS NOT NULL)
        OR (kind NOT IN ('virtual_machine', 'unikernel') AND cpu_model_kind IS NULL));
ALTER TABLE node_image ADD COLUMN vmx_enabled_kind INTEGER
    CHECK (((kind = 'virtual_machine' AND vmx_enabled_kind IS NOT NULL)
        OR (kind != 'virtual_machine' AND vmx_enabled_kind IS NULL))
        AND (vmx_enabled_kind IS NULL OR vmx_enabled_kind IN (0, 1)));
ALTER TABLE node_image ADD COLUMN machine_type_kind TEXT
    CHECK ((kind IN ('virtual_machine', 'unikernel') AND machine_type_kind IS NOT NULL)
        OR (kind NOT IN ('virtual_machine', 'unikernel') AND machine_type_kind IS NULL));
ALTER TABLE node_image ADD COLUMN bios_kind TEXT
    CHECK ((kind = 'virtual_machine' AND bios_kind IS NOT NULL)
        OR (kind != 'virtual_machine' AND bios_kind IS NULL));
ALTER TABLE node_image ADD COLUMN hdd_bus_kind TEXT
    CHECK ((kind = 'virtual_machine' AND hdd_bus_kind IS NOT NULL)
        OR (kind != 'virtual_machine' AND hdd_bus_kind IS NULL));
ALTER TABLE node_image ADD COLUMN cdrom_kind TEXT
    CHECK (cdrom_kind IS NULL OR kind = 'virtual_machine');
ALTER TABLE node_image ADD COLUMN cdrom_bus_kind TEXT
    CHECK ((kind = 'virtual_machine' AND cdrom_bus_kind IS NOT NULL)
        OR (kind != 'virtual_machine' AND cdrom_bus_kind IS NULL));
ALTER TABLE node_image ADD COLUMN interface_type_kind TEXT
    CHECK ((kind IN ('virtual_machine', 'unikernel') AND interface_type_kind IS NOT NULL)
        OR (kind NOT IN ('virtual_machine', 'unikernel') AND interface_type_kind IS NULL));
ALTER TABLE node_image ADD COLUMN interface_mtu_kind INTEGER
    CHECK (((kind IN ('virtual_machine', 'unikernel') AND interface_mtu_kind IS NOT NULL)
        OR (kind NOT IN ('virtual_machine', 'unikernel') AND interface_mtu_kind IS NULL))
        AND (interface_mtu_kind IS NULL OR interface_mtu_kind BETWEEN 576 AND 9600));
ALTER TABLE node_image ADD COLUMN ztp_kind TEXT
    CHECK ((kind IN ('virtual_machine', 'container') AND ztp_kind IS NOT NULL)
        OR (kind NOT IN ('virtual_machine', 'container') AND ztp_kind IS NULL));
ALTER TABLE node_image ADD COLUMN repo_kind TEXT
    CHECK (repo_kind IS NULL OR kind = 'container');
ALTER TABLE node_image ADD COLUMN boot_mode_kind TEXT
    CHECK ((kind = 'unikernel' AND boot_mode_kind IS NOT NULL)
        OR (kind != 'unikernel' AND boot_mode_kind IS NULL));

UPDATE node_image SET
    os_variant_kind = CASE WHEN kind = 'virtual_machine' THEN os_variant END,
    cpu_architecture_kind
        = CASE WHEN kind IN ('virtual_machine', 'unikernel') THEN cpu_architecture END,
    cpu_model_kind = CASE WHEN kind IN ('virtual_machine', 'unikernel') THEN cpu_model END,
    vmx_enabled_kind = CASE WHEN kind = 'virtual_machine' THEN vmx_enabled END,
    machine_type_kind = CASE WHEN kind IN ('virtual_machine', 'unikernel') THEN machine_type END,
    bios_kind = CASE WHEN kind = 'virtual_machine' THEN bios END,
    hdd_bus_kind = CASE WHEN kind = 'virtual_machine' THEN hdd_bus END,
    cdrom_kind = CASE WHEN kind = 'virtual_machine' THEN cdrom END,
    cdrom_bus_kind = CASE WHEN kind = 'virtual_machine' THEN cdrom_bus END,
    interface_type_kind
        = CASE WHEN kind IN ('virtual_machine', 'unikernel') THEN interface_type END,
    interface_mtu_kind
        = CASE WHEN kind IN ('virtual_machine', 'unikernel') THEN interface_mtu END,
    ztp_kind = CASE WHEN kind IN ('virtual_machine', 'container') THEN ztp END,
    repo_kind = CASE WHEN kind = 'container' THEN repo END,
    boot_mode_kind = CASE WHEN kind = 'unikernel' THEN coalesce(boot_mode, 'direct_kernel') END;

ALTER TABLE node_image DROP COLUMN os_variant;
ALTER TABLE node_image DROP COLUMN cpu_architecture;
ALTER TABLE node_image DROP COLUMN cpu_model;
ALTER TABLE node_image DROP COLUMN vmx_enabled;
ALTER TABLE node_image DROP COLUMN machine_type;
ALTER TABLE node_image DROP COLUMN bios;
ALTER TABLE node_image DROP COLUMN hdd_bus;
ALTER TABLE node_image DROP COLUMN cdrom;
ALTER TABLE node_image DROP COLUMN cdrom_bus;
ALTER TABLE node_image DROP COLUMN interface_type;
ALTER TABLE node_image DROP COLUMN interface_mtu;
ALTER TABLE node_image DROP COLUMN ztp;
ALTER TABLE node_image DROP COLUMN repo;
ALTER TABLE node_image DROP COLUMN boot_mode;
ALTER TABLE node_image DROP COLUMN interface_prefix;
ALTER TABLE node_image DROP COLUMN first_interface_index;

ALTER TABLE node_image RENAME COLUMN os_variant_kind TO os_variant;
ALTER TABLE node_image RENAME COLUMN cpu_architecture_kind TO cpu_architecture;
ALTER TABLE node_image RENAME COLUMN cpu_model_kind TO cpu_model;
ALTER TABLE node_image RENAME COLUMN vmx_enabled_kind TO vmx_enabled;
ALTER TABLE node_image RENAME COLUMN machine_type_kind TO machine_type;
ALTER TABLE node_image RENAME COLUMN bios_kind TO bios;
ALTER TABLE node_image RENAME COLUMN hdd_bus_kind TO hdd_bus;
ALTER TABLE node_image RENAME COLUMN cdrom_kind TO cdrom;
ALTER TABLE node_image RENAME COLUMN cdrom_bus_kind TO cdrom_bus;
ALTER TABLE node_image RENAME COLUMN interface_type_kind TO interface_type;
ALTER TABLE node_image RENAME COLUMN interface_mtu_kind TO interface_mtu;
ALTER TABLE node_image RENAME COLUMN ztp_kind TO ztp;
ALTER TABLE node_image RENAME COLUMN repo_kind TO repo;
ALTER TABLE node_image RENAME COLUMN boot_mode_kind TO boot_mode;
