//! Builtin node provider trait.

use gantz_ca::ContentAddr;

/// Trait for providing builtin (hard-coded) nodes.
///
/// Builtins are nodes that are always available and not stored in the registry.
/// They typically include primitive operations like arithmetic, control flow, etc.
///
/// This trait is object-safe when used as `dyn Builtins<Node = N>`.
pub trait Builtins: Send + Sync {
    /// The node type produced by this builtins provider.
    type Node: 'static + Send + Sync;

    /// Get all builtin node names.
    fn names(&self) -> Vec<&str>;

    /// Create a new instance of a builtin node by name.
    fn create(&self, name: &str) -> Option<Self::Node>;

    /// Get a builtin node instance by content address.
    fn instance(&self, ca: &ContentAddr) -> Option<&Self::Node>;

    /// Get the name of a builtin by content address.
    fn name(&self, ca: &ContentAddr) -> Option<&str>;

    /// Get content address by name.
    fn content_addr(&self, name: &str) -> Option<ContentAddr>;
}
