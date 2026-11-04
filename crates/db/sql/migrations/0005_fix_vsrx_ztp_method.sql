-- Corrects node images written with an old default: juniper_vsrxv3 defaulted to the TFTP ZTP
-- method, but vSRX's factory config runs no DHCP client, so it never fetched its config and booted
-- without one. vSRX reads its config from a CD-ROM ISO, as vMX and vQFX do. The importer also runs
-- it after writing node images from a server older than 0.5.0. Released migrations never change,
-- so the model and method names are written out.
UPDATE node_image SET ztp_method = 'cdrom'
WHERE model = 'juniper_vsrxv3' AND ztp_method = 'tftp';
