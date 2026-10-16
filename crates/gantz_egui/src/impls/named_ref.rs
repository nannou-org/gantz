use crate::node::gui::{GUI_REF_EXT_KEY, Gui, GuiDisplay, GuiRefExt, GuiRole};
use crate::node::{missing_color, outdated_color};
use crate::ui_tree::UiTree;
use crate::{
    BranchNode, ContextMenuResponse, InspectorRowsResponse, InspectorUiResponse, NodeCtx, NodeUi,
    NodeUiResponse, NodeViewResponse, OpenHead, ReplaceHead, SocketDoc,
    widget::node_inspector::{self, radio_option},
};
use gantz_core::node;
use gantz_edit::NamedRef;
use std::borrow::Cow;
use steel::SteelVal;

impl NodeUi for NamedRef {
    fn name(&self, _registry: &crate::Env<'_>) -> Cow<'_, str> {
        Cow::Owned(self.name().to_string())
    }

    fn demo_graph(&self, registry: &crate::Env<'_>) -> Option<String> {
        registry.demo_graph(&self.name().to_string())
    }

    fn nav_head(&self, _registry: &crate::Env<'_>) -> Option<gantz_ca::Head> {
        Some(gantz_ca::Head::Branch(self.name().clone()))
    }

    fn socket_doc(
        &self,
        registry: &crate::Env<'_>,
        kind: crate::SocketKind,
        ix: usize,
    ) -> Option<SocketDoc> {
        // Surface the referenced graph's inlet/outlet marker docs.
        registry.socket_doc(&self.content_addr(), kind, ix)
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let registry = ctx.env();
        let mut changed = false;

        // Nested graphs always sync so parents follow their children's edits.
        // `sync` is part of the CA, so flipping it on is a genuine edit. A
        // read-only ref makes neither silent edit.
        if !ctx.is_read_only() && self.is_nested() && !self.sync() {
            self.set_sync(true);
            changed = true;
        }

        // Auto-sync when enabled and the name points at newer content. No
        // widget is touched, but this silent mutation still changes the CA.
        if !ctx.is_read_only() && self.resync(|name| registry.name_ca(&name.to_string())) {
            changed = true;
        }

        // Recalculate after a possible sync.
        let name_str = self.name().to_string();
        let ref_ca = self.content_addr();
        let is_missing = !registry.node_exists(&ref_ca);
        let is_outdated = !is_missing
            && registry
                .name_ca(&name_str)
                .map(|ca| ca != ref_ca)
                .unwrap_or(false);

        // A healthy reference renders its marker tree in place of the name
        // label, per the resolved display mode.
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
        // in place. The focused tab navigates to it and the breadcrumb returns
        // to the parent. A reference to a root graph opens as a new tab.
        // Either way, the scene's open-in-new-tab context-menu action opens it
        // as a separate tab. See `nav_head`. Clicks consumed by marker body
        // widgets never reach this node-area response, so double-clicking a
        // dialer edits it rather than navigating.
        if resp.framed.inner.response.double_clicked() {
            let head = gantz_ca::Head::Branch(self.name().clone());
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
        let ref_ca = self.content_addr();
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
        // writes and pushes ride the payload channel, never `changed`.
        let registry = ctx.env();
        let ref_ca = self.content_addr();
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

        let is_missing = !registry.node_exists(&self.content_addr());

        // CA row.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("CA");
            });
            row.col(|ui| {
                let ca_string = format!("{}", self.content_addr().display_short());
                ui.add(egui::Label::new(egui::RichText::new(ca_string).monospace()));
            });
        });

        // Sync toggle row. Forced on and disabled for nested graphs.
        let nested = self.is_nested();
        if nested && !self.sync() {
            self.set_sync(true);
            resp.mark_changed();
        }
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("sync");
            });
            row.col(|ui| {
                let mut sync = self.sync();
                if nested {
                    ui.add_enabled(false, egui::Checkbox::new(&mut sync, ""))
                        .on_disabled_hover_text(
                            "sync is always on for nested graphs so the parent \
                             follows the child's edits",
                        );
                } else if ui
                    .checkbox(&mut sync, "")
                    .on_hover_text("automatically update to the latest commit")
                    .changed()
                {
                    self.set_sync(sync);
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
        // Status row for an outdated reference. Sync or fork resolves it.
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
        // stored as absence. An explicit pick is stored even when it equals
        // the default, because the user chose it. Ext writes are CA edits.
        if marker_of(&registry.gui_markers(&self.content_addr()), GuiRole::Body).is_some() {
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

        // Domain extension rows. See `RefExtUi`. Read them out of the ctx
        // first, since the accessor returns the ctx's own lifetime. The ctx
        // can then be passed down to each extension.
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

/// The display mode an instance renders with. The per-instance ext override
/// wins. Else the body marker's definition default. Else `Label` when the
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
/// `None` when the marker has no stored tree yet. That is when it is
/// unregistered, `Void`, or the read failed. Callers fall back to their
/// default rendering.
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
/// `path`. Output counts and ref-gui chains resolve through the environment
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

/// The name's current head graph CA when this reference is outdated. Outdated
/// means it exists, auto-sync is off, and the name points at different
/// content. `None` when missing, synced, or already up to date.
fn outdated_latest(named: &NamedRef, registry: &crate::Env<'_>) -> Option<gantz_ca::ContentAddr> {
    if named.sync() {
        return None;
    }
    let ref_ca = named.content_addr();
    if !registry.node_exists(&ref_ca) {
        return None;
    }
    match registry.name_ca(&named.name().to_string()) {
        Some(latest) if latest != ref_ca => Some(latest),
        _ => None,
    }
}

/// The outcome of [`sync_fork_buttons`], applied to the caller's response.
enum SyncForkAction {
    /// Neither button was clicked.
    None,
    /// `sync` was clicked. The reference was repointed, a CA-affecting edit.
    Synced,
    /// `fork` was clicked. Emit this [`BranchNode`] payload.
    Forked(BranchNode),
}

/// Render the `sync` and `fork` buttons for an outdated reference. `sync`
/// repoints the reference at `latest` by mutating `named`. `fork` produces a
/// [`BranchNode`] pinning a fresh name at the current outdated commit. The
/// inspector and the node context menu share it. Each applies the returned
/// [`SyncForkAction`] to its own response.
fn sync_fork_buttons(
    named: &mut NamedRef,
    path: &[node::Id],
    ui: &mut egui::Ui,
    latest: gantz_ca::ContentAddr,
) -> SyncForkAction {
    let current_short = named.content_addr().display_short().to_string();
    let latest_short = latest.display_short().to_string();

    let sync_hover = format!("sync reference from {current_short} to {latest_short}");
    if ui.button("sync").on_hover_text(sync_hover).clicked() {
        named.set_ref(named.ref_().retarget(latest));
        return SyncForkAction::Synced;
    }

    let fork_hover = format!("fork a new node at {current_short}");
    if ui.button("fork").on_hover_text(fork_hover).clicked() {
        let new_name = format!("{}-{}", named.name(), current_short);
        return SyncForkAction::Forked(BranchNode {
            new_name,
            ca: named.content_addr(),
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
        // With no ext, the body marker's definition default.
        assert_eq!(
            resolved_display(None, Some(GuiDisplay::Compact)),
            GuiDisplay::Compact,
        );
        // With no body marker at all, the label.
        assert_eq!(resolved_display(None, None), GuiDisplay::Label);
        // An ext override applies even without a marker default.
        assert_eq!(
            resolved_display(ext(GuiDisplay::Full), None),
            GuiDisplay::Full,
        );
    }

    /// A reference into an empty registry has a missing graph and no markers.
    /// It renders no marker tree, so the body falls back to the label.
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
        let graph = gantz_ca::DataGraph::default();
        let ctx = crate::NodeCtx::new(&env, &graph, &[0][..], &[], &[], &[], &mut vm, &mut writes);
        assert!(body_tree(&named, &env, &ctx).is_none());
    }

    /// An explicit display choice stores as ext even when it equals the
    /// definition default. `auto` is absence, not default-pruning. The ext
    /// write changes the node's stored erased address, and removal reverts
    /// it.
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
