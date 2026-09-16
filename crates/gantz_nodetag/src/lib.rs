//! The [`NodeTag`] trait: a stable wire tag identifying a node type.
//!
//! A tag identifies a node type independent of the serialization format.
//! The same tag identifies a node in `.gantz` text, in persisted registries
//! and in any other serde target. The trait lives in this leaf crate so that
//! every node-defining crate, including `gantz_core`, can depend on it and
//! declare its tag at the definition site.

/// A node type's wire tag. It is the value of the `"type"` entry in the
/// node's serialized map form. For example, `(node "Expr" ...)` in `.gantz`
/// text.
///
/// Declare the tag alongside the node type so that every application that
/// composes the node set agrees on the wire format. The derive of the same
/// name defaults the tag to the type's name and accepts a `#[tag("...")]`
/// override:
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
/// Tags are part of the wire format. Changing one breaks the loading of
/// existing exports and persisted registries that contain the node.
pub trait NodeTag {
    /// The `"type"` tag identifying this node type on the wire.
    const TAG: &'static str;
}

pub use gantz_nodetag_derive::NodeTag;
