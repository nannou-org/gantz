//! A node that references another node by name and content address.

use crate::{Cmd, NodeCtx, NodeUi, widget::node_inspector};
use gantz_core::node::{self, Node};
use serde::{Deserialize, Serialize};
use steel::steel_vm::engine::Engine;

/// The warning color used for outdated references.
pub fn outdated_color() -> egui::Color32 {
    egui::Color32::from_rgb(200, 150, 50)
}

/// A node that references another node by name and content address.
///
/// Similar to [`gantz_core::node::Ref`], but also stores the human-readable
/// name associated with the reference. This allows for detecting when the
/// name's current commit differs from the stored reference.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct NamedRef {
    /// The underlying reference by content address.
    ref_: gantz_core::node::Ref,
    /// The human-readable name associated with this reference.
    name: String,
    /// Whether to automatically sync to the latest commit.
    #[serde(default)]
    pub(crate) sync: bool,
}

/// Trait for environments that can check if a name maps to a content address.
pub trait NameRegistry {
    /// Returns the current content address for the given name, if it exists.
    fn name_ca(&self, name: &str) -> Option<gantz_ca::ContentAddr>;
}

impl NamedRef {
    /// Construct a `NamedRef` node.
    pub fn new(name: String, ref_: gantz_core::node::Ref) -> Self {
        Self {
            ref_,
            name,
            sync: false,
        }
    }

    /// The human-readable name associated with this reference.
    pub fn name(&self) -> &str {
        &self.name
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
}

impl<Env> Node<Env> for NamedRef
where
    Env: gantz_core::node::ref_::NodeRegistry,
    Env::Node: Node<Env>,
{
    fn n_inputs(&self, env: &Env) -> usize {
        self.ref_.n_inputs(env)
    }

    fn n_outputs(&self, env: &Env) -> usize {
        self.ref_.n_outputs(env)
    }

    fn branches(&self, env: &Env) -> Vec<node::EvalConf> {
        self.ref_.branches(env)
    }

    fn expr(&self, ctx: node::ExprCtx<Env>) -> node::ExprResult {
        self.ref_.expr(ctx)
    }

    fn push_eval(&self, env: &Env) -> Vec<node::EvalConf> {
        self.ref_.push_eval(env)
    }

    fn pull_eval(&self, env: &Env) -> Vec<node::EvalConf> {
        self.ref_.pull_eval(env)
    }

    fn stateful(&self, env: &Env) -> bool {
        <gantz_core::node::Ref as Node<Env>>::stateful(&self.ref_, env)
    }

    fn register(&self, env: &Env, path: &[node::Id], vm: &mut Engine) {
        <gantz_core::node::Ref as Node<Env>>::register(&self.ref_, env, path, vm)
    }

    fn inlet(&self, env: &Env) -> bool {
        <gantz_core::node::Ref as Node<Env>>::inlet(&self.ref_, env)
    }

    fn outlet(&self, env: &Env) -> bool {
        <gantz_core::node::Ref as Node<Env>>::outlet(&self.ref_, env)
    }

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        vec![self.ref_.content_addr()]
    }

    fn visit(&self, ctx: gantz_core::visit::Ctx<Env>, visitor: &mut dyn node::Visitor<Env>) {
        self.ref_.visit(ctx, visitor)
    }
}

impl gantz_ca::CaHash for NamedRef {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        "gantz_egui::node::NamedRef".hash(hasher);
        self.ref_.hash(hasher);
        self.name.hash(hasher);
    }
}

impl<Env> NodeUi<Env> for NamedRef
where
    Env: NameRegistry,
{
    fn name(&self, _env: &Env) -> &str {
        &self.name
    }

    fn ui(
        &mut self,
        ctx: NodeCtx<Env>,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        let env = ctx.env();
        let current_ca = env.name_ca(&self.name);
        let ref_ca = self.ref_.content_addr();
        let is_outdated = current_ca.map(|ca| ca != ref_ca).unwrap_or(false);

        // Auto-sync if enabled and outdated.
        if self.sync && is_outdated {
            if let Some(ca) = current_ca {
                self.ref_ = gantz_core::node::Ref::new(ca);
            }
        }

        // Recalculate after potential sync.
        let is_outdated = env
            .name_ca(&self.name)
            .map(|ca| ca != self.ref_.content_addr())
            .unwrap_or(false);

        // Regular frame, warning color only on name if outdated.
        let response = uictx.framed(|ui| {
            let name_text = if is_outdated {
                egui::RichText::new(&self.name).color(outdated_color())
            } else {
                egui::RichText::new(&self.name)
            };
            ui.add(egui::Label::new(name_text).selectable(false))
        });

        // Open the node on double-click (handler decides if the node is openable).
        if response.response.double_clicked() {
            ctx.cmds.push(Cmd::OpenNamedNode(
                self.name.clone(),
                self.ref_.content_addr(),
            ));
        }

        response
    }

    fn inspector_rows(&mut self, ctx: &mut NodeCtx<Env>, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        let env = ctx.env();
        let current_ca = env.name_ca(&self.name);
        let is_outdated = current_ca
            .map(|ca| ca != self.ref_.content_addr())
            .unwrap_or(false);

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

        // Sync toggle row.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("sync");
            });
            row.col(|ui| {
                ui.checkbox(&mut self.sync, "")
                    .on_hover_text("automatically update to the latest commit");
            });
        });

        // Status row - only shown when sync is disabled and outdated.
        if !self.sync && is_outdated {
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
                                ctx.cmds.push(Cmd::ForkNamedNode {
                                    new_name: new_name.clone(),
                                    ca: self.ref_.content_addr(),
                                });
                                self.name = new_name;
                            }
                        });
                    });
                });
            }
        }
    }
}
