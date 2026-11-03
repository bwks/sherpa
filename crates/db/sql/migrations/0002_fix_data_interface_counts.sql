-- Corrects node images written with an old default: cisco_csr1000v and cisco_iosvl2 once defaulted
-- to 16 data interfaces, one more than they have after the management interface, so `sherpa up`
-- failed on them. The importer also runs it after writing node images from a server older than
-- 0.5.0. Released migrations never change, so the model names are written out.
UPDATE node_image SET data_interface_count = 15
WHERE model IN ('cisco_csr1000v', 'cisco_iosvl2') AND data_interface_count > 15;
