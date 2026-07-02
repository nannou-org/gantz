//! The [`NodeTag`] trait: a stable wire tag identifying a node type.
//!
//! Tagging node types consistently is independent of any particular
//! serialization format - the same tag identifies a node in `.gantz` text,
//! persisted registries, or any other serde target - so the trait lives in
//! this dedicated leaf crate that every node-defining crate (including
//! `gantz_core`) can depend on, letting each node declare its tag at its own
//! definition site.

/// A node type's wire tag: the value of the `"type"` entry in its serialized
/// map form, e.g. `(node "Expr" ...)` in `.gantz` text.
///
/// Declared alongside the node type itself (like its `#[cahash(...)]`
/// discriminator) so that every application composing the node set agrees on
/// the same wire format. Usually implemented via the derive of the same
/// name, which defaults the tag to the type's name and takes a
/// `#[tag("...")]` override:
///
/// ```
/// use gantz_nodetag::NodeTag;
///
/// #[derive(NodeTag)]
/// struct Gain;
///
/// #[derive(NodeTag)]
/// #[tag("gain.custom")]
/// struct CustomGain;
///
/// assert_eq!(Gain::TAG, "Gain");
/// assert_eq!(CustomGain::TAG, "gain.custom");
/// ```
///
/// Tags are part of the wire format: changing one breaks the loading of
/// existing exports and persisted registries that contain the node.
pub trait NodeTag {
    /// The `"type"` tag identifying this node type on the wire.
    const TAG: &'static str;
}

pub use gantz_nodetag_derive::NodeTag;
