//! A node that references another node by name and content address.

use crate::{Cmd, NodeCtx, NodeUi, widget::node_inspector};
use gantz_core::node::{self, Node};
use serde::{Deserialize, Serialize};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

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
}

/// Trait for environments that can check if a name maps to a content address.
pub trait NameRegistry {
    /// Returns the current content address for the given name, if it exists.
    fn name_ca(&self, name: &str) -> Option<gantz_ca::ContentAddr>;
}

impl NamedRef {
    /// Construct a `NamedRef` node.
    pub fn new(name: String, ref_: gantz_core::node::Ref) -> Self {
        Self { ref_, name }
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

    fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
        self.ref_.expr(ctx)
    }

    fn push_eval(&self, env: &Env) -> Vec<node::EvalConf> {
        self.ref_.push_eval(env)
    }

    fn pull_eval(&self, env: &Env) -> Vec<node::EvalConf> {
        self.ref_.pull_eval(env)
    }

    fn stateful(&self) -> bool {
        <gantz_core::node::Ref as Node<Env>>::stateful(&self.ref_)
    }

    fn register(&self, path: &[node::Id], vm: &mut Engine) {
        <gantz_core::node::Ref as Node<Env>>::register(&self.ref_, path, vm)
    }

    fn inlet(&self) -> bool {
        <gantz_core::node::Ref as Node<Env>>::inlet(&self.ref_)
    }

    fn outlet(&self) -> bool {
        <gantz_core::node::Ref as Node<Env>>::outlet(&self.ref_)
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

        // Use a slightly different frame stroke when outdated.
        let response = if is_outdated {
            let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(200, 150, 50));
            let frame = egui::Frame::group(uictx.style()).stroke(stroke);
            uictx.framed_with(frame, |ui| {
                ui.horizontal(|ui| {
                    let warn =
                        egui::RichText::new("!").color(egui::Color32::from_rgb(200, 150, 50));
                    let warn_res = ui.label(warn);
                    let name_res = ui.add(egui::Label::new(&self.name).selectable(false));
                    warn_res.union(name_res)
                })
                .inner
            })
        } else {
            uictx.framed(|ui| ui.add(egui::Label::new(&self.name).selectable(false)))
        };

        // Open the node on double-click (handler decides if the node is openable).
        if response.response.double_clicked() {
            ctx.cmds.push(Cmd::OpenNamedNode(self.name.clone(), ref_ca));
        }

        response
    }

    fn inspector_rows(&mut self, ctx: &NodeCtx<Env>, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());

        // Show content address.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("CA");
            });
            row.col(|ui| {
                let ca_string = format!("{}", self.ref_.content_addr().display_short());
                ui.add(egui::Label::new(egui::RichText::new(ca_string).monospace()));
            });
        });

        // Show update button if outdated.
        let env = ctx.env();
        if let Some(current_ca) = env.name_ca(&self.name) {
            if current_ca != self.ref_.content_addr() {
                body.row(row_h, |mut row| {
                    row.col(|ui| {
                        ui.label("Status");
                    });
                    row.col(|ui| {
                        ui.horizontal(|ui| {
                            let warn_text = egui::RichText::new("outdated")
                                .color(egui::Color32::from_rgb(200, 150, 50));
                            ui.label(warn_text);
                            if ui.button("Update").clicked() {
                                self.ref_ = gantz_core::node::Ref::new(current_ca);
                            }
                        });
                    });
                });
            }
        }
    }
}
