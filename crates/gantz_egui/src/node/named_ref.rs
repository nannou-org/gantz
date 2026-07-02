//! A node that references another node by name and content address.

use crate::{
    BranchNode, ContextMenuResponse, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse,
    OpenHead, ReplaceHead, SocketDoc, widget::node_inspector,
};
use gantz_ca::CaHash;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, Node, RegCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// The warning color used for outdated references.
pub fn outdated_color() -> egui::Color32 {
    egui::Color32::from_rgb(200, 150, 50)
}

/// The error color used for missing references.
pub fn missing_color() -> egui::Color32 {
    egui::Color32::from_rgb(200, 80, 80)
}

/// A node that references another node by name and content address.
///
/// Similar to [`gantz_core::node::Ref`], but also stores the human-readable
/// name associated with the reference. This allows for detecting when the
/// name's current commit differs from the stored reference.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash, NodeTag)]
#[cahash("gantz.named-ref")]
pub struct NamedRef {
    /// The underlying reference by content address.
    ref_: gantz_core::node::Ref,
    /// The human-readable name associated with this reference.
    name: String,
    /// Whether to automatically sync to the latest commit.
    ///
    /// Part of the content address: toggling it is a genuine edit, so the
    /// change rides the normal commit + export pipeline and persists (rather
    /// than being silently dropped by the registry's content-addressed dedup).
    #[serde(default)]
    pub(crate) sync: bool,
}

/// Trait for environments that can check if a name maps to a content address.
pub trait NameRegistry {
    /// Returns the current content address for the given name, if it exists.
    fn name_ca(&self, name: &str) -> Option<gantz_ca::ContentAddr>;
    /// Returns true if a node with the given content address exists in the environment.
    fn node_exists(&self, ca: &gantz_ca::ContentAddr) -> bool;
}

/// The separator reserved for nested-graph names (`parent:child`).
///
/// A `NamedRef` whose name contains this character is a *nested* graph: it is
/// hidden from the root graph-select list and its `sync` toggle is forced on so
/// edits to the child always propagate back to its parent.
pub const NESTED_SEP: char = ':';

impl NamedRef {
    /// Construct a `NamedRef` node (auto-sync disabled).
    pub fn new(name: String, ref_: gantz_core::node::Ref) -> Self {
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
    pub fn with_sync(name: String, ref_: gantz_core::node::Ref) -> Self {
        Self {
            ref_,
            name,
            sync: true,
        }
    }

    /// The human-readable name associated with this reference.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether this reference names a nested graph (`parent:child`).
    pub fn is_nested(&self) -> bool {
        self.name.contains(NESTED_SEP)
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

    /// Re-point this reference at a renamed target: change the stored name and
    /// repoint at the renamed graph's commit. Used by the rename cascade so a
    /// renamed parent keeps referencing its (also-renamed) children.
    pub fn rename(&mut self, name: String, ca: gantz_ca::ContentAddr) {
        self.name = name;
        self.ref_ = gantz_core::node::Ref::new(ca);
    }

    /// Bring the reference up to date with the name's current commit.
    ///
    /// When sync is enabled and `resolve(name)` differs from the current
    /// reference, the reference is repointed at the resolved address. Returns
    /// `true` if the reference changed. This is the single implementation shared
    /// by the inspector UI and the headless propagation pass.
    pub fn resync(&mut self, resolve: impl Fn(&str) -> Option<gantz_ca::ContentAddr>) -> bool {
        if !self.sync {
            return false;
        }
        match resolve(&self.name) {
            Some(ca) if ca != self.ref_.content_addr() => {
                self.ref_ = gantz_core::node::Ref::new(ca);
                true
            }
            _ => false,
        }
    }
}

impl crate::sync::AsNamedRef for NamedRef {
    fn as_named_ref(&self) -> Option<&NamedRef> {
        Some(self)
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

    fn visit(&self, ctx: gantz_core::visit::Ctx<'_, '_>, visitor: &mut dyn node::Visitor) {
        self.ref_.visit(ctx, visitor)
    }
}

impl NodeUi for NamedRef {
    fn name(&self, _registry: &dyn crate::Registry) -> &str {
        &self.name
    }

    fn demo_graph<'a>(&self, registry: &'a dyn crate::Registry) -> Option<&'a str> {
        registry.demo_graph(&self.name)
    }

    fn nav_head(&self, _registry: &dyn crate::Registry) -> Option<gantz_ca::Head> {
        Some(gantz_ca::Head::Branch(self.name.clone()))
    }

    fn socket_doc(
        &self,
        registry: &dyn crate::Registry,
        kind: crate::SocketKind,
        ix: usize,
    ) -> Option<SocketDoc> {
        // Surface the referenced graph's inlet/outlet marker docs.
        registry.socket_doc(&self.ref_.content_addr(), kind, ix)
    }

    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let registry = ctx.registry();
        let mut changed = false;

        // Nested graphs always sync so parents follow their children's edits.
        // Flipping the (CA-relevant) `sync` flag on is a genuine edit.
        if self.is_nested() && !self.sync {
            self.sync = true;
            changed = true;
        }

        // Auto-sync if enabled and the name points at a newer commit. This is a
        // silent mutation (no widget touched) that still changes the node's CA.
        if self.resync(|name| registry.name_ca(name)) {
            changed = true;
        }

        // Recalculate after potential sync.
        let ref_ca = self.ref_.content_addr();
        let is_missing = !registry.node_exists(&ref_ca);
        let is_outdated = !is_missing
            && registry
                .name_ca(&self.name)
                .map(|ca| ca != ref_ca)
                .unwrap_or(false);

        // Regular frame, error color if missing, warning color if outdated.
        let framed = uictx.framed(|ui, _sockets| {
            let name_text = if is_missing {
                egui::RichText::new(&self.name).color(missing_color())
            } else if is_outdated {
                egui::RichText::new(&self.name).color(outdated_color())
            } else {
                egui::RichText::new(&self.name)
            };
            ui.add(egui::Label::new(name_text).selectable(false))
        });

        let mut resp = NodeUiResponse::new(framed);
        resp.set_changed(changed);

        // Enter the referenced graph on double-click. A nested graph is entered
        // *in place* (the focused tab navigates to it; the breadcrumb returns to
        // the parent); a reference to a root graph opens as a new tab. Either
        // way, the scene's "open in new tab" context-menu action (see
        // `nav_head`) opens it as a separate tab.
        if resp.framed.inner.response.double_clicked() {
            let head = gantz_ca::Head::Branch(self.name.clone());
            if self.is_nested() {
                resp.emit(ReplaceHead(head));
            } else {
                resp.emit(OpenHead(head));
            }
        }

        resp
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        let row_h = node_inspector::table_row_h(body.ui_mut());
        let registry = ctx.registry();
        let path = ctx.path().to_vec();

        // Whether the referenced CA exists in the registry.
        let is_missing = !registry.node_exists(&self.ref_.content_addr());

        // CA row.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("CA");
            });
            row.col(|ui| {
                let ca_string = format!("{}", self.ref_.content_addr().display_short());
                ui.add(egui::Label::new(egui::RichText::new(ca_string).monospace()));
            });
        });

        // Sync toggle row. Forced on (and disabled) for nested graphs.
        let nested = self.is_nested();
        if nested && !self.sync {
            self.sync = true;
            resp.mark_changed();
        }
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("sync");
            });
            row.col(|ui| {
                if nested {
                    ui.add_enabled(false, egui::Checkbox::new(&mut self.sync, ""))
                        .on_disabled_hover_text(
                            "sync is always on for nested graphs so the parent \
                             follows the child's edits",
                        );
                } else if ui
                    .checkbox(&mut self.sync, "")
                    .on_hover_text("automatically update to the latest commit")
                    .changed()
                {
                    resp.mark_changed();
                }
            });
        });

        // Status row for missing CA.
        if is_missing {
            body.row(row_h, |mut row| {
                row.col(|ui| {
                    ui.label("status");
                });
                row.col(|ui| {
                    let err_text = egui::RichText::new("missing").color(missing_color());
                    ui.label(err_text);
                });
            });
        // Status row for an outdated reference - sync/fork to resolve it.
        } else if let Some(latest_ca) = outdated_latest(self, registry) {
            body.row(row_h, |mut row| {
                row.col(|ui| {
                    ui.label("status");
                });
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        let warn_text = egui::RichText::new("outdated").color(outdated_color());
                        ui.label(warn_text);
                        match sync_fork_buttons(self, &path, ui, latest_ca) {
                            SyncForkAction::Synced => resp.mark_changed(),
                            SyncForkAction::Forked(branch) => resp.emit(branch),
                            SyncForkAction::None => {}
                        }
                    });
                });
            });
        }
        resp
    }

    fn context_menu(&mut self, ctx: &mut NodeCtx, ui: &mut egui::Ui) -> ContextMenuResponse {
        let mut resp = ContextMenuResponse::default();
        // Offer sync/fork on the node itself when the reference is outdated.
        if let Some(latest_ca) = outdated_latest(self, ctx.registry()) {
            let path = ctx.path().to_vec();
            match sync_fork_buttons(self, &path, ui, latest_ca) {
                SyncForkAction::Synced => {
                    resp.mark_changed();
                    ui.close();
                }
                SyncForkAction::Forked(branch) => {
                    resp.emit(branch);
                    ui.close();
                }
                SyncForkAction::None => {}
            }
        }
        resp
    }
}

/// The name's current commit CA when this reference is *outdated*: it exists,
/// auto-sync is off, and the name now points at a different commit. `None`
/// otherwise (missing, synced, or already up to date).
fn outdated_latest(
    named: &NamedRef,
    registry: &dyn crate::Registry,
) -> Option<gantz_ca::ContentAddr> {
    if named.sync {
        return None;
    }
    let ref_ca = named.ref_.content_addr();
    if !registry.node_exists(&ref_ca) {
        return None;
    }
    match registry.name_ca(&named.name) {
        Some(latest) if latest != ref_ca => Some(latest),
        _ => None,
    }
}

/// The outcome of [`sync_fork_buttons`], applied to the caller's response.
enum SyncForkAction {
    /// Neither button was clicked.
    None,
    /// `sync` was clicked: the reference was repointed (a CA-affecting edit).
    Synced,
    /// `fork` was clicked: emit this [`BranchNode`] payload.
    Forked(BranchNode),
}

/// Render the `sync` and `fork` buttons for an outdated reference. `sync`
/// repoints the reference at `latest` (mutating `named`); `fork` produces a
/// [`BranchNode`] pinning a fresh name at the current (outdated) commit. Shared
/// by the inspector and the node context menu, which apply the returned
/// [`SyncForkAction`] to their own response (`changed` / emitted payload).
fn sync_fork_buttons(
    named: &mut NamedRef,
    path: &[node::Id],
    ui: &mut egui::Ui,
    latest: gantz_ca::ContentAddr,
) -> SyncForkAction {
    let current_short = named.ref_.content_addr().display_short().to_string();
    let latest_short = latest.display_short().to_string();

    let sync_hover = format!("sync reference from {current_short} to {latest_short}");
    if ui.button("sync").on_hover_text(sync_hover).clicked() {
        named.ref_ = gantz_core::node::Ref::new(latest);
        return SyncForkAction::Synced;
    }

    let fork_hover = format!("fork a new node at {current_short}");
    if ui.button("fork").on_hover_text(fork_hover).clicked() {
        let new_name = format!("{}-{}", named.name, current_short);
        return SyncForkAction::Forked(BranchNode {
            new_name,
            ca: named.ref_.content_addr(),
            path: path.to_vec(),
        });
    }

    SyncForkAction::None
}
