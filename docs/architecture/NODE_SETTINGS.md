# Node Settings

A node's settings come from its image definition (`shared::data::NodeConfig`) and its manifest
node's overrides (`topology::Node`). Both group them the same way and nest by group on every wire.
The keys are in sherpa-docs `manifest.md`; the behaviour is in `tests/specs/manifests.md` and
`images.md`.

## Model and kind

A model takes one or more kinds; `NodeModel::kinds()` is an exhaustive `match`, so a new model
does not compile without them, and the first is the default. `generic` has none, so its node or
image request must name a kind. `shared::data::ModelKind` is a model with a kind it takes: its only
constructor refuses any other pair, JSON included, so everything that depends on both (built-in
image, ZTP methods, ready port, interfaces, the `node_ops` helpers) takes one and needs no check.
The guest account rules stay per model, as they are about the OS. Images, their locks and their
routes are keyed by model, kind and version.

`NodeModel` is read only through `FromStr`, from serde and clap alike, so a name is accepted or
refused the same way everywhere; the names in `shared::data::REMOVED_MODELS` are known only to be
refused with their replacement. No code matches model names by prefix or substring.

## Image definition

`NodeConfig` holds the settings every kind takes and exactly one kind's group: `settings` is a
`KindSettings` enum, so an image cannot hold another kind's settings. Each setting sits in the
group of the kinds whose code reads it. The constructors start from a base in `node_base.rs` and
state only what differs; no model is built from another, so a base change reaches only its models.

## Manifest node

`topology::Node` is one struct for every kind, every setting optional. The hardware groups live in
`shared::data::performance` because the `template` crate, which depends only on `shared`, renders
them. The kind groups are `Option`s, so an empty one on the wrong kind is still refused, and the
kind is never inferred from a group. `NodeConfig`, `Node`, every group and the image forms
`deny_unknown_fields`, so an old or misplaced key fails naming it; nothing maps old keys to new.
Which kind takes which setting is one table, `shared::data::kind_settings`, which the manifest
check, the image JSON check and the image editor all use.

## Which kinds take what

A container's `cpu.count` and `memory` are Docker limits, set only when the manifest sets them, as
several image values are placeholders. A CPU quota holds no host CPUs, so containers take no part
in pinning. A `juniper_evolved` container's limit is its EVO VM's memory plus
`konst::JUNIPER_EVOLVED_CONTAINER_OVERHEAD_MIB`, with no quota, which would throttle that VM; IOL's
is its memory plus `CISCO_IOL_CONTAINER_OVERHEAD_MIB`.
