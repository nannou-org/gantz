//! Decode diagnostics.
//!
//! Issues fall on one of two sides of the boundary rule in the crate doc.
//!
//! - An [`ErrorReason`], carried by [`crate::elem::ErrorElem`], means the
//!   subtree cannot render meaningfully. The element keeps its slot in the
//!   tree so a host can render an inline error chip while siblings render
//!   normally.
//! - A [`Warning`] means the element still renders after recovering, for
//!   example by ignoring an unknown attribute or falling back to a
//!   documented default.

use thiserror::Error;

/// A path of child element indices from the tree root.
///
/// Indices address the `children` vectors of decoded elements, so they count
/// element positions after the tag, any attribute block and any positional
/// arguments of the parent form.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct TreePath(pub Vec<usize>);

/// A non-fatal decode note attached to the element at `path`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Warning {
    /// The element the note is attached to.
    pub path: TreePath,
    /// What was recovered from and how.
    pub kind: WarningKind,
}

/// A recoverable decode issue. The element renders regardless.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum WarningKind {
    /// An attribute the element does not know. Ignored for forward
    /// compatibility.
    #[error("unknown attribute `{attr}` on `{tag}`")]
    UnknownAttr {
        /// The element's tag.
        tag: String,
        /// The unknown attribute's name.
        attr: String,
    },
    /// An attribute value of the wrong shape. The field falls back to its
    /// default.
    #[error("attribute `{attr}` on `{tag}` expects {expected}, found {found}")]
    InvalidAttrValue {
        /// The element's tag.
        tag: String,
        /// The attribute's name.
        attr: String,
        /// What the attribute accepts.
        expected: &'static str,
        /// A summary of the value found.
        found: String,
    },
    /// The same attribute given more than once. The first value wins.
    #[error("duplicate attribute `{attr}` on `{tag}`, first value wins")]
    DuplicateAttr {
        /// The element's tag.
        tag: String,
        /// The duplicated attribute's name.
        attr: String,
    },
    /// An attribute entry that is not a `(name value)` pair. Skipped.
    #[error("malformed attribute entry on `{tag}`, expected a (name value) pair, found {found}")]
    MalformedAttr {
        /// The element's tag.
        tag: String,
        /// A summary of the entry found.
        found: String,
    },
    /// A required attribute was absent and a default was substituted.
    #[error("`{tag}` expects attribute `{attr}`, defaulting to {default}")]
    MissingAttr {
        /// The element's tag.
        tag: String,
        /// The missing attribute's name.
        attr: String,
        /// The substituted default, rendered for the message.
        default: String,
    },
    /// A positional argument of the wrong shape. A default was substituted.
    #[error("`{tag}` expects {what}, found {found}")]
    InvalidArg {
        /// The element's tag.
        tag: String,
        /// What the argument accepts.
        what: &'static str,
        /// A summary of the value found.
        found: String,
    },
    /// Items given to an element that takes no children. Ignored.
    #[error("`{tag}` takes no children, ignored {count}")]
    IgnoredChildren {
        /// The element's tag.
        tag: String,
        /// How many items were ignored.
        count: usize,
    },
    /// Text expected but absent. An empty string was substituted.
    #[error("`{tag}` expects {what}, none found")]
    MissingText {
        /// The element's tag.
        tag: String,
        /// What kind of text was expected.
        what: &'static str,
    },
}

/// Why a subtree decoded to an inline error element.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ErrorReason {
    /// A tag outside the vocabulary.
    #[error("unknown element `{0}`")]
    UnknownTag(String),
    /// A tag claimed by the vocabulary for a future version.
    #[error("`{0}` is reserved for a future version")]
    ReservedTag(String),
    /// A value in element position that is not an element form.
    #[error("expected an element list, found {found}")]
    NotAnElement {
        /// A summary of the value found.
        found: String,
    },
    /// An attribute block anywhere other than immediately after the tag.
    #[error("an attribute block is only valid immediately after the tag")]
    MisplacedAttrs,
    /// A required positional argument absent or of the wrong shape.
    #[error("`{tag}` requires {what}, found {found}")]
    MissingArg {
        /// The element's tag.
        tag: String,
        /// What the argument accepts.
        what: &'static str,
        /// A summary of what was found, or "nothing".
        found: String,
    },
    /// Nesting deeper than the decoder's depth limit.
    #[error("depth limit of {0} exceeded")]
    DepthLimit(usize),
    /// More elements than the decoder's element limit. The remaining
    /// siblings of the slot holding this error were dropped.
    #[error("element limit of {0} exceeded, remaining siblings dropped")]
    ElementLimit(usize),
}
