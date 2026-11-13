-- Container and unikernel images carry machine, boot disk and CD-ROM settings that nothing reads
-- for them. Images stored before this version could hold any value there: the four network OS
-- containers were defined with a q35 machine and SATA buses, and the web form writes whatever it
-- is given. Each is reset to the defaults new images carry, so every image of a kind agrees.
-- Released migrations never change, so the kind and value names are written out.
--
-- A container uses none of them.
UPDATE node_image
SET machine_type = 'pc', bios = 'sea_bios', hdd_bus = 'virtio', cdrom = NULL, cdrom_bus = 'virtio'
WHERE kind = 'container';
-- A unikernel runs on its machine type, which is kept, and uses none of the others.
UPDATE node_image
SET bios = 'sea_bios', hdd_bus = 'virtio', cdrom = NULL, cdrom_bus = 'virtio'
WHERE kind = 'unikernel';
