-- Corrects node images written with an old default: sonic_linux defaulted to 52 data interfaces,
-- but the SONiC virtual switch has 32 front-panel ports, so the VM got 20 NICs that map to no
-- port and a link on eth33 to eth52 went nowhere. Only images still at the old default change; a
-- count an administrator set is kept. Released migrations never change, so the model name is
-- written out.
UPDATE node_image SET data_interface_count = 32
WHERE model = 'sonic_linux' AND data_interface_count = 52;
