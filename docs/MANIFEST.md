# Manifest Reference

Sherpa lab manifests are TOML files.

## Per-node data interfaces

A node can override the number of data interfaces created for that instance with
`data_interface_count`:

```toml
name = "example-lab"

nodes = [
  { name = "dev01", model = "ubuntu_linux", data_interface_count = 4 },
  { name = "dev02", model = "ubuntu_linux", data_interface_count = 4 },
]

links = [
  { src = "dev01::eth4", dst = "dev02::eth4" },
]
```

`data_interface_count` counts data interfaces only. It does not include the
management interface or any reserved interfaces required by the node model.

If omitted, Sherpa uses the default interface count from the node image/model
configuration. Overrides are validated against the interface names supported by
the selected model, so requesting more interfaces than the model can name will
fail manifest validation.
