//! Sugar keyword <-> typetag tag mapping for the `.gantz` text format.
//!
//! Each entry pairs a human-friendly node keyword with the typetag `"type"`
//! tag of the underlying node. The `ref`/`fn-ref` (references), `graph`
//! (inline nesting) and `node` (generic fallback) forms are handled
//! separately by the reader/writer and are not listed here.

/// Sugar keyword -> typetag tag, for node specs that lower to a plain serde
/// object. Order is the canonical display order for documentation only.
pub const KEYWORD_TAG: &[(&str, &str)] = &[
    ("inlet", "Inlet"),
    ("outlet", "Outlet"),
    ("apply", "Apply"),
    ("delay", "Delay"),
    ("id", "Identity"),
    ("bang", "Bang"),
    ("add", "Add"),
    ("inspect", "Inspect"),
    ("frame-bang", "FrameBang"),
    ("number", "Number"),
    ("log", "Log"),
    ("expr", "Expr"),
    ("branch", "Branch"),
    ("comment", "Comment"),
];

/// The typetag tag for a sugar keyword.
pub fn tag_for_keyword(kw: &str) -> Option<&'static str> {
    KEYWORD_TAG
        .iter()
        .find(|(k, _)| *k == kw)
        .map(|&(_, tag)| tag)
}

/// The sugar keyword for a typetag tag, if one exists.
pub fn keyword_for_tag(tag: &str) -> Option<&'static str> {
    KEYWORD_TAG
        .iter()
        .find(|(_, t)| *t == tag)
        .map(|&(kw, _)| kw)
}
