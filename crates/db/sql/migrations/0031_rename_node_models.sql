-- Node models renamed in 0.7.0. Model names drop a "v" or "c" where the name left is still the
-- vendor's name for the software, and models that were one product in two kinds became one
-- model with two kinds: `arista_veos` and `arista_ceos` are `arista_eos`, `juniper_vevolved`
-- and `juniper_cevolved` are `juniper_evolved`, and the three `generic_*` models are `generic`.
--
-- Only `node_image.model` names a model. A node refers to its image, and a lab's stored
-- definition takes the node's model from that image, so running labs and their definitions
-- follow this update with no other row changed. Each row keeps its kind, so the two rows of a
-- merged model cannot collide on (model, kind, version), and each kind keeps its default.
-- A container image keeps the repo it was pulled or imported as, so Docker is not touched.
--
-- Released migrations never change, so the model names are written out.
UPDATE node_image SET model = 'arista_eos' WHERE model IN ('arista_veos', 'arista_ceos');
UPDATE node_image SET model = 'cisco_asa' WHERE model = 'cisco_asav';
UPDATE node_image SET model = 'cisco_ftd' WHERE model = 'cisco_ftdv';
UPDATE node_image SET model = 'cisco_iosxr' WHERE model = 'cisco_iosxrv9000';
UPDATE node_image SET model = 'juniper_router' WHERE model = 'juniper_vrouter';
UPDATE node_image SET model = 'juniper_switch' WHERE model = 'juniper_vswitch';
UPDATE node_image SET model = 'juniper_evolved'
WHERE model IN ('juniper_vevolved', 'juniper_cevolved');
UPDATE node_image SET model = 'juniper_srx' WHERE model = 'juniper_vsrxv3';
UPDATE node_image SET model = 'generic'
WHERE model IN ('generic_vm', 'generic_container', 'generic_unikernel');
