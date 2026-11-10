//! The rule that keeps exactly one default image per model and kind.
//!
//! Every write that can change an image's `default` flag (import, download, pull, scan, the admin
//! form, set-default and delete) decides the flag here, inside the transaction that writes it.
//! The partial unique index `node_image_one_default` refuses a second default; this rule keeps a
//! model that has images from having none.

use std::error::Error;
use std::fmt;

use anyhow::{Context, Result};
use shared::data::{NodeConfig, NodeKind, NodeModel};
use turso::Connection;

use crate::persistence::{enum_text, node_image_from_row};
use crate::queries;
use crate::sql::query_optional;

/// What a write asks for one version's `default` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultRequest {
    /// Make this version the default, clearing the flag on the others.
    Make,
    /// Leave the flag as it is. A version the model has no default without becomes the default.
    Keep,
    /// Stop this version being the default. Refused when it is the default, since that would
    /// leave the model without one; another version must be made the default instead.
    Unset,
}

/// Where a model's default stands, seen from the version being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DefaultHolder {
    /// The version being written is the default.
    ThisVersion,
    /// Another version is the default.
    OtherVersion,
    /// The model has no default, as when it has no images yet.
    Nobody,
}

impl DefaultHolder {
    /// Where the default stands for `version`, given the model's current default image.
    pub(crate) fn of(version: &str, default: Option<&NodeConfig>) -> Self {
        match default {
            Some(image) if image.version == version => Self::ThisVersion,
            Some(_) => Self::OtherVersion,
            None => Self::Nobody,
        }
    }
}

/// The `default` flag a version gets when `request` is applied while `holder` holds the default.
/// `None` when the request is refused: unsetting the default would leave the model without one.
pub(crate) fn resolve_default(holder: DefaultHolder, request: DefaultRequest) -> Option<bool> {
    match (holder, request) {
        (_, DefaultRequest::Make)
        | (DefaultHolder::Nobody, _)
        | (DefaultHolder::ThisVersion, DefaultRequest::Keep) => Some(true),
        (DefaultHolder::ThisVersion, DefaultRequest::Unset) => None,
        (DefaultHolder::OtherVersion, DefaultRequest::Keep | DefaultRequest::Unset) => Some(false),
    }
}

/// Apply `request` to `version` of `model` and `kind` inside the caller's transaction, and return
/// the `default` flag the version must be written with. When that is `true`, the flag is already
/// cleared on the model's other versions, so the caller's write cannot meet the unique index.
///
/// # Errors
///
/// Fails with [`LastDefaultImage`] when the request would unset the model's only default, and when
/// the database cannot be read or written.
pub(crate) async fn claim_default(
    conn: &Connection,
    model: NodeModel,
    kind: &NodeKind,
    version: &str,
    request: DefaultRequest,
) -> Result<bool> {
    let model_text = enum_text(&model, "model")?;
    let kind_text = enum_text(kind, "kind")?;
    let current = query_optional(
        conn,
        queries::node_image::FIND_DEFAULT,
        (model_text.as_str(), kind_text.as_str()),
        node_image_from_row,
    )
    .await
    .with_context(|| format!("Failed to read the default image of {model} ({kind})"))?;
    let holder = DefaultHolder::of(version, current.as_ref());
    let is_default = resolve_default(holder, request).ok_or_else(|| LastDefaultImage {
        model,
        kind: kind.clone(),
        version: version.to_owned(),
    })?;
    if is_default && holder == DefaultHolder::OtherVersion {
        conn.execute(
            queries::node_image::CLEAR_DEFAULT_EXCEPT_VERSION,
            (model_text.as_str(), kind_text.as_str(), version),
        )
        .await
        .with_context(|| format!("Failed to clear the default image of {model} ({kind})"))?;
    }
    Ok(is_default)
}

/// An edit tried to unset a model's only default image. Make another version the default instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastDefaultImage {
    pub model: NodeModel,
    pub kind: NodeKind,
    pub version: String,
}

impl fmt::Display for LastDefaultImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Version '{}' is the default image for {} ({}); make another version the default instead",
            self.version, self.model, self.kind
        )
    }
}

impl Error for LastDefaultImage {}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLDERS: [DefaultHolder; 3] = [
        DefaultHolder::ThisVersion,
        DefaultHolder::OtherVersion,
        DefaultHolder::Nobody,
    ];

    #[test]
    fn make_always_takes_the_default() {
        for holder in HOLDERS {
            assert_eq!(
                resolve_default(holder, DefaultRequest::Make),
                Some(true),
                "{holder:?}"
            );
        }
    }

    #[test]
    fn keep_never_takes_the_default_from_another_version_nor_gives_it_up() {
        assert_eq!(
            resolve_default(DefaultHolder::ThisVersion, DefaultRequest::Keep),
            Some(true)
        );
        assert_eq!(
            resolve_default(DefaultHolder::OtherVersion, DefaultRequest::Keep),
            Some(false)
        );
    }

    #[test]
    fn a_model_without_a_default_takes_the_version_written() {
        for request in [
            DefaultRequest::Make,
            DefaultRequest::Keep,
            DefaultRequest::Unset,
        ] {
            assert_eq!(
                resolve_default(DefaultHolder::Nobody, request),
                Some(true),
                "{request:?}"
            );
        }
    }

    #[test]
    fn the_default_cannot_be_unset_only_replaced() {
        assert_eq!(
            resolve_default(DefaultHolder::ThisVersion, DefaultRequest::Unset),
            None
        );
        assert_eq!(
            resolve_default(DefaultHolder::OtherVersion, DefaultRequest::Unset),
            Some(false)
        );
    }

    #[test]
    fn the_holder_is_read_from_the_current_default() {
        let mut image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        image.version = "24.04".to_owned();
        assert_eq!(
            DefaultHolder::of("24.04", Some(&image)),
            DefaultHolder::ThisVersion
        );
        assert_eq!(
            DefaultHolder::of("22.04", Some(&image)),
            DefaultHolder::OtherVersion
        );
        assert_eq!(DefaultHolder::of("22.04", None), DefaultHolder::Nobody);
    }
}
