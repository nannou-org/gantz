//! A node that references another node by name and content address.

use crate::{BranchNode, NodeCtx, NodeUi, OpenHead, ReplaceHead, widget::node_inspector};
use gantz_ca::CaHash;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, Node, RegCtx};
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
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.named-ref")]
pub struct NamedRef {
    /// The underlying reference by content address.
    ref_: gantz_core::node::Ref,
    /// The human-readable name associated with this reference.
    name: String,
    /// Whether to automatically sync to the latest commit.
    #[serde(default)]
    #[cahash(skip)]
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
        registry.demo_graph(&self.ref_.content_addr())
    }

    fn nav_head(&self, _registry: &dyn crate::Registry) -> Option<gantz_ca::Head> {
        Some(gantz_ca::Head::Branch(self.name.clone()))
    }

    fn ui(
        &mut self,
        mut ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        let registry = ctx.registry();

        // Nested graphs always sync so parents follow their children's edits.
        if self.is_nested() {
            self.sync = true;
        }

        // Auto-sync if enabled and the name points at a newer commit.
        self.resync(|name| registry.name_ca(name));

        // Recalculate after potential sync.
        let ref_ca = self.ref_.content_addr();
        let is_missing = !registry.node_exists(&ref_ca);
        let is_outdated = !is_missing
            && registry
                .name_ca(&self.name)
                .map(|ca| ca != ref_ca)
                .unwrap_or(false);

        // Regular frame, error color if missing, warning color if outdated.
        let response = uictx.framed(|ui, _sockets| {
            let name_text = if is_missing {
                egui::RichText::new(&self.name).color(missing_color())
            } else if is_outdated {
                egui::RichText::new(&self.name).color(outdated_color())
            } else {
                egui::RichText::new(&self.name)
            };
            ui.add(egui::Label::new(name_text).selectable(false))
        });

        // Enter the referenced graph on double-click. A nested graph is entered
        // *in place* (the focused tab navigates to it; the breadcrumb returns to
        // the parent); a reference to a root graph opens as a new tab. Either
        // way, the scene's "open in new tab" context-menu action (see
        // `nav_head`) opens it as a separate tab.
        if response.inner.response.double_clicked() {
            let head = gantz_ca::Head::Branch(self.name.clone());
            if self.is_nested() {
                ctx.response(ReplaceHead(head));
            } else {
                ctx.response(OpenHead(head));
            }
        }

        response
    }

    fn inspector_rows(&mut self, ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        let registry = ctx.registry();
        let ref_ca = self.ref_.content_addr();

        // Check if the referenced CA exists in registry.
        let is_missing = !registry.node_exists(&ref_ca);

        // Check if outdated (name points to different CA).
        let current_ca = registry.name_ca(&self.name);
        let is_outdated = !is_missing && current_ca.map(|ca| ca != ref_ca).unwrap_or(false);

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
        if nested {
            self.sync = true;
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
                } else {
                    ui.checkbox(&mut self.sync, "")
                        .on_hover_text("automatically update to the latest commit");
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
        // Status row for outdated CA - only shown when sync is disabled.
        } else if !self.sync && is_outdated {
            if let Some(latest_ca) = current_ca {
                let current_short = self.ref_.content_addr().display_short().to_string();
                let latest_short = latest_ca.display_short().to_string();

                body.row(row_h, |mut row| {
                    row.col(|ui| {
                        ui.label("status");
                    });
                    row.col(|ui| {
                        ui.horizontal(|ui| {
                            let warn_text = egui::RichText::new("outdated").color(outdated_color());
                            ui.label(warn_text);

                            let sync_hover = format!(
                                "sync reference from {} to {}",
                                current_short, latest_short
                            );
                            if ui.button("sync").on_hover_text(sync_hover).clicked() {
                                self.ref_ = gantz_core::node::Ref::new(latest_ca);
                            }

                            let fork_hover = format!("fork a new node at {}", current_short);
                            if ui.button("fork").on_hover_text(fork_hover).clicked() {
                                let new_name = format!("{}-{}", self.name, current_short);
                                ctx.response(BranchNode {
                                    new_name,
                                    ca: self.ref_.content_addr(),
                                    path: ctx.path.to_vec(),
                                });
                            }
                        });
                    });
                });
            }
        }
    }
}
