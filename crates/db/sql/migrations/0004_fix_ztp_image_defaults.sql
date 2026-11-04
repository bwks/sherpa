-- Corrects node images written with old defaults that ZTP cannot use, so `sherpa up` failed on
-- them. The importer also runs it after writing node images from a server older than 0.5.0.
-- Released migrations never change, so the model, method and bus names are written out.
--
-- cisco_ise defaulted to the CD-ROM ZTP method, which has no ISE generator; ISE reads its ZTP
-- config from a disk.
UPDATE node_image SET ztp_method = 'disk'
WHERE model = 'cisco_ise' AND ztp_method = 'cdrom';
-- generic_vm defaulted to a VirtIO CD-ROM bus, which cannot hold the cloud-init ISO.
UPDATE node_image SET cdrom_bus = 'sata'
WHERE model = 'generic_vm' AND cdrom_bus = 'virtio';
