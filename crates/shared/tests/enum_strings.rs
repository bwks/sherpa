//! Each enum the web UI's forms list by `Display` and read back through serde must give the same
//! string both ways, or a saved form is refused or stores another value.

use std::fmt::Display;
use std::str::FromStr;

use serde::Serialize;
use shared::data::{
    BiosTypes, BridgeKind, CpuArchitecture, CpuModels, DiskBuses, InterfaceType, MachineType,
    NodeKind, NodeModel, OsVariant, ZtpMethod,
};
use strum::IntoEnumIterator;

/// Every variant of `T` displays as its serde string.
fn assert_display_is_serde<T: IntoEnumIterator + Display + Serialize>() {
    for variant in T::iter() {
        let serde = serde_json::to_value(&variant).unwrap();
        assert_eq!(
            serde.as_str(),
            Some(variant.to_string().as_str()),
            "{} displays as {variant} but serializes as {serde}",
            std::any::type_name::<T>()
        );
    }
}

#[test]
fn form_enums_display_as_they_serialize() {
    assert_display_is_serde::<OsVariant>();
    assert_display_is_serde::<BiosTypes>();
    assert_display_is_serde::<CpuArchitecture>();
    assert_display_is_serde::<CpuModels>();
    assert_display_is_serde::<MachineType>();
    assert_display_is_serde::<DiskBuses>();
    assert_display_is_serde::<ZtpMethod>();
    assert_display_is_serde::<InterfaceType>();
    assert_display_is_serde::<NodeKind>();
    assert_display_is_serde::<BridgeKind>();
}

#[test]
fn every_node_model_round_trips_through_display_serde_and_from_str() {
    assert_display_is_serde::<NodeModel>();
    for model in NodeModel::iter() {
        assert_eq!(NodeModel::from_str(&model.to_string()).unwrap(), model);
    }
}
