//! A node that references another node by name and content address.

use crate::node::gui::{GUI_REF_EXT_KEY, Gui, GuiDisplay, GuiRefExt, GuiRole};
use crate::ui_tree::UiTree;
use crate::{
    BranchNode, ContextMenuResponse, InspectorRowsResponse, InspectorUiResponse, NodeCtx, NodeUi,
    NodeUiResponse, NodeViewResponse, OpenHead, ReplaceHead, SocketDoc,
    widget::node_inspector::{self, radio_option},
};
use gantz_ca::Name;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, Node, RegCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use steel::SteelVal;

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
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
pub struct NamedRef {
    /// The underlying reference by content address.
    ref_: gantz_core::node::Ref,
    /// The human-readable name associated with this reference.
    name: Name,
    /// Whether to automatically sync to the latest commit.
    ///
    /// Part of the content address: toggling it is a genuine edit, so the
    /// change rides the normal commit + export pipeline and persists (rather
    /// than being silently dropped by the registry's content-addressed dedup).
    #[serde(default, skip_serializing_if = "is_default")]
    pub(crate) sync: bool,
}

fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == T::default()
}

impl NamedRef {
    /// Construct a `NamedRef` node (auto-sync disabled).
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

    /// Whether this reference names a nested graph (`parent:child`).
    ///
    /// A `NamedRef` naming a nested graph is hidden from the root
    /// graph-select list and its `sync` toggle is forced on so edits to the
    /// child always propagate back to its parent.
    pub fn is_nested(&self) -> bool {
        self.name.is_nested()
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
    /// [`Ref`](gantz_core::node::Ref). See [`gantz_core::node::Ref::ext_as`].
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

    /// Re-point this reference at a renamed target: change the stored name and
    /// repoint at the renamed graph's head graph address. Used by the rename
    /// cascade so a renamed parent keeps referencing its (also-renamed)
    /// children.
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

impl NodeUi for NamedRef {
    fn name(&self, _registry: &crate::Env<'_>) -> Cow<'_, str> {
        Cow::Owned(self.name.to_string())
    }

    fn demo_graph(&self, registry: &crate::Env<'_>) -> Option<String> {
        registry.demo_graph(&self.name.to_string())
    }

    fn nav_head(&self, _registry: &crate::Env<'_>) -> Option<gantz_ca::Head> {
        Some(gantz_ca::Head::Branch(self.name.clone()))
    }

    fn socket_doc(
        &self,
        registry: &crate::Env<'_>,
        kind: crate::SocketKind,
        ix: usize,
    ) -> Option<SocketDoc> {
        // Surface the referenced graph's inlet/outlet marker docs.
        registry.socket_doc(&self.ref_.content_addr(), kind, ix)
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let registry = ctx.env();
        let mut changed = false;

        // Nested graphs always sync so parents follow their children's edits.
        // Flipping the (CA-relevant) `sync` flag on is a genuine edit.
        if self.is_nested() && !self.sync {
            self.sync = true;
            changed = true;
        }

        // Auto-sync if enabled and the name points at newer content. This is a
        // silent mutation (no widget touched) that still changes the node's CA.
        if self.resync(|name| registry.name_ca(&name.to_string())) {
            changed = true;
        }

        // Recalculate after potential sync.
        let name_str = self.name.to_string();
        let ref_ca = self.ref_.content_addr();
        let is_missing = !registry.node_exists(&ref_ca);
        let is_outdated = !is_missing
            && registry
                .name_ca(&name_str)
                .map(|ca| ca != ref_ca)
                .unwrap_or(false);

        // A healthy reference renders its marker tree (per the resolved
        // display mode) in place of the name label.
        let tree = (!is_missing && !is_outdated)
            .then(|| body_tree(self, registry, &ctx))
            .flatten();

        let mut payloads = Vec::new();
        let framed = match tree {
            Some(decoded) => {
                let path = ctx.path();
                let (n_outputs, ref_gui) = resolvers(registry, ref_ca, path);
                let root_id = uictx.egui_id().with("gui");
                uictx.framed(|ui, _sockets| {
                    let r = UiTree::new(root_id)
                        .instance_prefix(path)
                        .n_outputs(&n_outputs)
                        .ref_gui(&ref_gui)
                        .show(&decoded.root, &mut ctx, ui);
                    payloads = r.payloads;
                    r.inner.unwrap_or_else(|| ui.response())
                })
            }
            // Regular frame, error color if missing, warning color if
            // outdated.
            None => uictx.framed(|ui, _sockets| {
                let name_text = if is_missing {
                    egui::RichText::new(&name_str).color(missing_color())
                } else if is_outdated {
                    egui::RichText::new(&name_str).color(outdated_color())
                } else {
                    egui::RichText::new(&name_str)
                };
                ui.add(egui::Label::new(name_text).selectable(false))
            }),
        };

        let mut resp = NodeUiResponse::new(framed);
        resp.set_changed(changed);
        resp.payloads.extend(payloads);

        // Enter the referenced graph on double-click. A nested graph is entered
        // *in place* (the focused tab navigates to it; the breadcrumb returns to
        // the parent); a reference to a root graph opens as a new tab. Either
        // way, the scene's "open in new tab" context-menu action (see
        // `nav_head`) opens it as a separate tab. Clicks consumed by marker
        // body widgets never reach this node-area response, so double-clicking
        // e.g. a dialer edits it rather than navigating.
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

    fn view_ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
        // The view marker, else the body marker, else the default state view.
        let registry = ctx.env();
        let ref_ca = self.ref_.content_addr();
        let markers = registry.gui_markers(&ref_ca);
        let marker =
            marker_of(&markers, GuiRole::View).or_else(|| marker_of(&markers, GuiRole::Body));
        let Some(decoded) = marker.and_then(|(ix, _)| marker_tree(&ctx, ix)) else {
            return crate::default_view_ui(&ctx, ui);
        };
        let path = ctx.path();
        let (n_outputs, ref_gui) = resolvers(registry, ref_ca, path);
        let r = UiTree::new(ui.id().with("gui"))
            .instance_prefix(path)
            .n_outputs(&n_outputs)
            .ref_gui(&ref_gui)
            .show(&decoded.root, &mut ctx, ui);
        let mut resp = NodeViewResponse::default();
        resp.inner = r.inner;
        resp.payloads = r.payloads;
        resp
    }

    fn inspector_ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> InspectorUiResponse {
        // The inspector marker tree renders after the default table. State
        // writes and pushes ride the payload channel; never `changed`.
        let registry = ctx.env();
        let ref_ca = self.ref_.content_addr();
        let marker = marker_of(&registry.gui_markers(&ref_ca), GuiRole::Inspector);
        let Some(decoded) = marker.and_then(|(ix, _)| marker_tree(&ctx, ix)) else {
            return InspectorUiResponse::default();
        };
        let path = ctx.path();
        let (n_outputs, ref_gui) = resolvers(registry, ref_ca, path);
        let r = UiTree::new(ui.id().with("gui-inspector"))
            .instance_prefix(path)
            .n_outputs(&n_outputs)
            .ref_gui(&ref_gui)
            .show(&decoded.root, &mut ctx, ui);
        let mut resp = InspectorUiResponse::default();
        resp.inner = r.inner;
        resp.payloads = r.payloads;
        resp
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        let row_h = node_inspector::table_row_h(body.ui_mut());
        let registry = ctx.env();
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

        // GUI display override row, shown when the referenced graph declares
        // a body marker. `auto` follows the graph's definition default and is
        // stored as *absence*; an explicit pick is stored even when it equals
        // the default (the user chose it). Ext writes are CA edits.
        if marker_of(
            &registry.gui_markers(&self.ref_.content_addr()),
            GuiRole::Body,
        )
        .is_some()
        {
            let mut choice: Option<GuiDisplay> =
                self.ext_as::<GuiRefExt>(GUI_REF_EXT_KEY).map(|e| e.display);
            body.row(row_h, |mut row| {
                row.col(|ui| {
                    ui.label("gui");
                });
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        let mut edited = radio_option(
                            ui,
                            &mut choice,
                            None,
                            "auto",
                            "follow the graph's display default",
                        );
                        for display in GuiDisplay::ALL {
                            let hover = match display {
                                GuiDisplay::Full => "render the full body tree",
                                GuiDisplay::Compact => "render the compact tree",
                                GuiDisplay::Label => "render the name label",
                            };
                            edited |= radio_option(
                                ui,
                                &mut choice,
                                Some(display),
                                display.as_str(),
                                hover,
                            );
                        }
                        if edited {
                            match choice {
                                None => {
                                    self.remove_ext(GUI_REF_EXT_KEY);
                                }
                                Some(display) => self
                                    .set_ext(GUI_REF_EXT_KEY, &GuiRefExt { display })
                                    .expect("`GuiRefExt` is datum-representable"),
                            }
                            resp.mark_changed();
                        }
                    });
                });
            });
        }

        // Domain extension rows (see `RefExtUi`). Read out of the ctx first
        // (the accessor returns the ctx's own lifetime) so the ctx can be
        // passed down to each extension.
        let ext_uis = ctx.ref_ext_uis();
        for ext_ui in ext_uis {
            let inner = ext_ui.inspector_rows(self, ctx, body);
            resp.set_changed(inner.changed);
            resp.payloads.extend(inner.payloads);
        }
        resp
    }

    fn context_menu(&mut self, ctx: &mut NodeCtx, ui: &mut egui::Ui) -> ContextMenuResponse {
        let mut resp = ContextMenuResponse::default();
        // Offer sync/fork on the node itself when the reference is outdated.
        if let Some(latest_ca) = outdated_latest(self, ctx.env()) {
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

/// The display mode an instance renders with: the per-instance ext override,
/// else the body marker's definition default, else `Label` when the
/// referenced graph declares no body marker.
fn resolved_display(ext: Option<GuiRefExt>, body_display: Option<GuiDisplay>) -> GuiDisplay {
    match ext {
        Some(ext) => ext.display,
        None => body_display.unwrap_or(GuiDisplay::Label),
    }
}

/// The first marker of `role`, in index order.
fn marker_of(markers: &[(node::Id, Gui)], role: GuiRole) -> Option<(node::Id, Gui)> {
    markers.iter().copied().find(|(_, gui)| gui.role == role)
}

/// Read and decode the stored tree of the marker at `ctx.path() ++ [ix]`.
///
/// `None` when the marker has no stored tree yet (unregistered, `Void`, or a
/// failed read) - callers fall back to their default rendering.
fn marker_tree(ctx: &NodeCtx, ix: node::Id) -> Option<gantz_ui::Decoded> {
    let path: Vec<node::Id> = ctx.path().iter().copied().chain(Some(ix)).collect();
    match ctx.extract_value_at(&path) {
        Ok(Some(val)) if !matches!(val, SteelVal::Void) => Some(gantz_ui::codec::steel::decode(
            &val,
            &gantz_ui::Limits::default(),
        )),
        _ => None,
    }
}

/// The marker tree this instance's body renders in place of its name label,
/// resolved per the display mode. `None` falls back to the label.
fn body_tree(
    named: &NamedRef,
    registry: &crate::Env<'_>,
    ctx: &NodeCtx,
) -> Option<gantz_ui::Decoded> {
    let markers = registry.gui_markers(&named.content_addr());
    let body = marker_of(&markers, GuiRole::Body);
    let display = resolved_display(
        named.ext_as(GUI_REF_EXT_KEY),
        body.map(|(_, gui)| gui.display),
    );
    let (ix, _) = match display {
        GuiDisplay::Full => body?,
        GuiDisplay::Compact => marker_of(&markers, GuiRole::Compact)?,
        GuiDisplay::Label => return None,
    };
    marker_tree(ctx, ix)
}

/// The interpreter resolvers for an instance of the graph at `ca` rendered at
/// `path`: output counts and ref-gui chains resolve through the environment
/// relative to the referenced graph.
fn resolvers<'a>(
    registry: &'a crate::Env<'a>,
    ca: gantz_ca::ContentAddr,
    path: &'a [node::Id],
) -> (
    impl Fn(&[node::Id]) -> Option<usize> + 'a,
    impl Fn(&[node::Id]) -> Option<node::Id> + 'a,
) {
    let n_outputs = move |p: &[node::Id]| {
        let rel = p.strip_prefix(path)?;
        crate::reg::n_outputs_at(registry, &ca, rel)
    };
    let ref_gui = move |chain: &[node::Id]| {
        let (_, marker) = crate::reg::resolve_ref_chain(registry, ca, chain)?;
        Some(marker)
    };
    (n_outputs, ref_gui)
}

/// The name's current head graph CA when this reference is *outdated*: it
/// exists, auto-sync is off, and the name now points at different content.
/// `None` otherwise (missing, synced, or already up to date).
fn outdated_latest(named: &NamedRef, registry: &crate::Env<'_>) -> Option<gantz_ca::ContentAddr> {
    if named.sync {
        return None;
    }
    let ref_ca = named.ref_.content_addr();
    if !registry.node_exists(&ref_ca) {
        return None;
    }
    match registry.name_ca(&named.name.to_string()) {
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
        named.ref_ = named.ref_.retarget(latest);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_resolution_prefers_ext_then_marker_then_label() {
        let ext = |display| Some(GuiRefExt { display });
        // The ext override wins over the marker's default.
        assert_eq!(
            resolved_display(ext(GuiDisplay::Label), Some(GuiDisplay::Full)),
            GuiDisplay::Label,
        );
        // No ext: the body marker's definition default.
        assert_eq!(
            resolved_display(None, Some(GuiDisplay::Compact)),
            GuiDisplay::Compact,
        );
        // No body marker at all: the label.
        assert_eq!(resolved_display(None, None), GuiDisplay::Label);
        // An ext override applies even without a marker default.
        assert_eq!(
            resolved_display(ext(GuiDisplay::Full), None),
            GuiDisplay::Full,
        );
    }

    /// A reference into an empty registry (missing graph, no markers) renders
    /// no marker tree: the body falls back to the label.
    #[test]
    fn missing_ref_keeps_label() {
        let registry = gantz_ca::Registry::default();
        let graphs = gantz_core::data::ReifiedGraphs::new();
        let builtins = gantz_core::Builtins::default();
        let instances = crate::node::UiBuiltins::default();
        let codec = crate::test_node::codec();
        let env = crate::Env {
            registry: &registry,
            builtins: &builtins,
            codec: &codec,
            graphs: &graphs,
            instances: &instances,
        };
        let mut vm = gantz_core::steel::steel_vm::engine::Engine::new_base();
        let named = NamedRef::new(
            "missing".parse().unwrap(),
            gantz_core::node::Ref::new([0u8; 32].into()),
        );
        let mut writes = Vec::new();
        let ctx = crate::NodeCtx::new(&env, &[0][..], &[], &[], &[], &mut vm, &mut writes);
        assert!(body_tree(&named, &env, &ctx).is_none());
    }

    /// An explicit display choice stores as ext even when it equals the
    /// definition default (`auto` is absence, not default-pruning), and the
    /// ext write changes the node's stored (erased) address, reverting on
    /// removal.
    #[test]
    fn explicit_display_override_round_trips_through_ext() {
        let addr = |named: &NamedRef| {
            gantz_core::data::erase_node_typed(named)
                .expect("erase")
                .content_addr()
        };
        let mut named = NamedRef::new(
            "child".parse().unwrap(),
            gantz_core::node::Ref::new([0u8; 32].into()),
        );
        assert!(named.ext_as::<GuiRefExt>(GUI_REF_EXT_KEY).is_none());
        let ca_auto = addr(&named);

        let full = GuiRefExt {
            display: GuiDisplay::Full,
        };
        named.set_ext(GUI_REF_EXT_KEY, &full).unwrap();
        assert_eq!(named.ext_as::<GuiRefExt>(GUI_REF_EXT_KEY), Some(full));
        assert_ne!(addr(&named), ca_auto);

        named.remove_ext(GUI_REF_EXT_KEY);
        assert!(named.ext_as::<GuiRefExt>(GUI_REF_EXT_KEY).is_none());
        assert_eq!(addr(&named), ca_auto);
    }
}
