//! A node that references another node by name and content address.

use gantz_ca::{Name, NodeData};
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, Node, RegCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// A node that references another node by name and content address.
///
/// Similar to [`gantz_core::node::Ref`], but also stores the human-readable
/// name associated with the reference. This lets the editor detect when the
/// name's current commit differs from the stored reference.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
pub struct NamedRef {
    /// The underlying reference by content address.
    ref_: gantz_core::node::Ref,
    /// The human-readable name associated with this reference.
    name: Name,
    /// Whether to automatically sync to the latest commit.
    ///
    /// Part of the content address. Toggling it is a genuine edit, so the
    /// change rides the normal commit and export pipeline and persists. The
    /// registry's content-addressed dedup would otherwise drop it silently.
    #[serde(default, skip_serializing_if = "is_default")]
    sync: bool,
}

impl NamedRef {
    /// Construct a `NamedRef` node with auto-sync disabled.
    pub fn new(name: Name, ref_: gantz_core::node::Ref) -> Self {
        Self {
            ref_,
            name,
            sync: false,
        }
    }

    /// Construct a `NamedRef` node with auto-sync enabled.
    ///
    /// Used for nested graphs, whose parent must always follow the child's
    /// latest commit.
    pub fn with_sync(name: Name, ref_: gantz_core::node::Ref) -> Self {
        Self {
            ref_,
            name,
            sync: true,
        }
    }

    /// The human-readable name associated with this reference.
    pub fn name(&self) -> &Name {
        &self.name
    }

    /// Whether this reference names a nested graph, written `parent:child`.
    ///
    /// A `NamedRef` naming a nested graph is hidden from the root
    /// graph-select list. Its `sync` toggle is forced on so edits to the
    /// child always propagate back to its parent.
    pub fn is_nested(&self) -> bool {
        self.name.is_nested()
    }

    /// Whether the reference follows the name's latest commit. See
    /// [`NamedRef::resync`].
    pub fn sync(&self) -> bool {
        self.sync
    }

    /// Turn following the name's latest commit on or off.
    pub fn set_sync(&mut self, sync: bool) {
        self.sync = sync;
    }

    /// The underlying reference.
    pub fn ref_(&self) -> &gantz_core::node::Ref {
        &self.ref_
    }

    /// The content address of the referenced node.
    pub fn content_addr(&self) -> gantz_ca::ContentAddr {
        self.ref_.content_addr()
    }

    /// Update the reference to a new content address.
    pub fn set_ref(&mut self, ref_: gantz_core::node::Ref) {
        self.ref_ = ref_;
    }

    /// Decode the extension value stored under `key` on the underlying
    /// [`gantz_core::node::Ref`]. See [`gantz_core::node::Ref::ext_as`].
    pub fn ext_as<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.ref_.ext_as(key)
    }

    /// Store `value` as the underlying reference's extension data under `key`.
    /// See [`gantz_core::node::Ref::set_ext`].
    pub fn set_ext(
        &mut self,
        key: impl Into<String>,
        value: &impl serde::Serialize,
    ) -> Result<(), gantz_core::datum::DatumError> {
        self.ref_.set_ext(key, value)
    }

    /// Remove and return the underlying reference's extension datum stored
    /// under `key`, if any. See [`gantz_core::node::Ref::remove_ext`].
    pub fn remove_ext(&mut self, key: &str) -> Option<gantz_core::datum::Datum> {
        self.ref_.remove_ext(key)
    }

    /// Re-point this reference at a renamed target. Change the stored name and
    /// repoint at the renamed graph's head graph address. The rename cascade
    /// uses it so a renamed parent keeps referencing its renamed children.
    pub fn rename(&mut self, name: Name, ca: gantz_ca::ContentAddr) {
        self.name = name;
        self.ref_ = self.ref_.retarget(ca);
    }

    /// Bring the reference up to date with the name's current head graph.
    ///
    /// When sync is enabled and `resolve(name)` differs from the current
    /// reference, the reference is repointed at the resolved address. Returns
    /// `true` if the reference changed. This is the single implementation shared
    /// by the inspector UI and the headless propagation pass.
    pub fn resync(&mut self, resolve: impl Fn(&Name) -> Option<gantz_ca::ContentAddr>) -> bool {
        if !self.sync {
            return false;
        }
        match resolve(&self.name) {
            Some(ca) if ca != self.ref_.content_addr() => {
                self.ref_ = self.ref_.retarget(ca);
                true
            }
            _ => false,
        }
    }
}

// Declared on the wrapped type. The orphan rule forbids implementing
// `NodeTag` for `Fn<NamedRef>` here, since both are foreign.
impl gantz_core::node::FnNodeTag for NamedRef {
    const FN_TAG: &'static str = "FnNamedRef";
}

impl gantz_core::node::AsRefNode for NamedRef {
    fn as_ref_node(&self) -> Option<&gantz_core::node::Ref> {
        Some(&self.ref_)
    }
}

impl Node for NamedRef {
    fn n_inputs(&self, ctx: MetaCtx) -> usize {
        self.ref_.n_inputs(ctx)
    }

    fn n_outputs(&self, ctx: MetaCtx) -> usize {
        self.ref_.n_outputs(ctx)
    }

    fn branches(&self, ctx: MetaCtx) -> Vec<node::EvalConf> {
        self.ref_.branches(ctx)
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        self.ref_.expr(ctx)
    }

    fn push_eval(&self, ctx: MetaCtx) -> Vec<node::EvalConf> {
        self.ref_.push_eval(ctx)
    }

    fn pull_eval(&self, ctx: MetaCtx) -> Vec<node::EvalConf> {
        self.ref_.pull_eval(ctx)
    }

    fn stateful(&self, ctx: MetaCtx) -> bool {
        self.ref_.stateful(ctx)
    }

    fn register(&self, ctx: RegCtx<'_, '_>) {
        self.ref_.register(ctx)
    }

    fn inlet(&self, ctx: MetaCtx) -> bool {
        self.ref_.inlet(ctx)
    }

    fn outlet(&self, ctx: MetaCtx) -> bool {
        self.ref_.outlet(ctx)
    }

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        vec![self.ref_.content_addr()]
    }

    fn required_modules(&self, ctx: MetaCtx) -> Vec<String> {
        self.ref_.required_modules(ctx)
    }

    fn visit(&self, ctx: gantz_core::visit::Ctx<'_, '_>, visitor: &mut dyn node::Visitor) {
        self.ref_.visit(ctx, visitor)
    }
}

/// The [`NamedRef`] stored in `weight`, if it is one.
///
/// Tag-gated. Only a weight whose tag is exactly `NamedRef`'s matches. `Fn`
/// and `FnNamedRef` reference a graph without standing in for it, so they
/// never match. A tag-matched weight that fails to decode is logged and read
/// as `None`.
pub fn named_ref_of(weight: &NodeData) -> Option<NamedRef> {
    if weight.tag != <NamedRef as NodeTag>::TAG {
        return None;
    }
    match gantz_core::data::reify_node_concrete::<NamedRef>(weight) {
        Ok(named_ref) => Some(named_ref),
        Err(e) => {
            log::error!("failed to decode a stored `NamedRef`: {e}");
            None
        }
    }
}

/// Run `f` over the [`NamedRef`] stored in `weight`, writing the rewritten
/// node back when `f` reports a change. Returns whether `weight` changed.
///
/// Tag-gated like [`named_ref_of`]. The edit goes through
/// [`edit_node_typed`], which recomputes the refs column and keeps data that
/// this build does not recognise. Failures are logged and reported as
/// unchanged.
///
/// [`edit_node_typed`]: gantz_core::data::edit_node_typed
pub fn with_named_ref_mut(weight: &mut NodeData, f: impl FnOnce(&mut NamedRef) -> bool) -> bool {
    use gantz_core::data::{EditNodeError, edit_node_typed};
    if weight.tag != <NamedRef as NodeTag>::TAG {
        return false;
    }
    match edit_node_typed(weight, f) {
        Ok(changed) => changed,
        Err(EditNodeError::Lossy(lost)) => {
            log::warn!(
                "a `NamedRef` holds data this gantz does not recognise ({}), so it keeps its \
                 reference",
                lost.join(", "),
            );
            false
        }
        Err(e) => {
            log::error!("failed to edit a stored `NamedRef`: {e}");
            false
        }
    }
}

fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == T::default()
}
