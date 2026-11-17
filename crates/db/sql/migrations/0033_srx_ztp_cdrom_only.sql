-- juniper_srxv3 takes its config from a CD-ROM only (#676): TFTP is no longer a method it
-- takes. A vSRX on TFTP stalled at boot without its config, and `up` now refuses an image left on
-- `tftp`. Each such image moves to `cdrom`, the model's default.
--
-- The other values the column can hold for the model are kept: `cdrom` is already right, and
-- `disabled`, ZTP off, is a setting every model takes. Sherpa never wrote any other method for
-- the model, since every write is checked against the methods it takes.
--
-- Released migrations never change, so the model and method names are written out.
UPDATE node_image SET ztp = 'cdrom'
WHERE model = 'juniper_srxv3' AND ztp = 'tftp';
