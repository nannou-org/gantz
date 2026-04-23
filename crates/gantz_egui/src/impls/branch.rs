use crate::{NodeCtx, NodeUi};

/// A widget used to allow for editing and parsing a branch expression.
pub struct BranchEdit<'a> {
    branch: &'a mut gantz_core::node::Branch,
    pub id: egui::Id,
}

#[derive(Clone, Default)]
struct BranchEditState {
    branch_hash: u64,
    code: String,
}

impl<'a> egui::Widget for BranchEdit<'a> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let Self { branch, id } = self;
        let code_id = id.with("code");

        // Retrieve the working state.
        let mut state: BranchEditState = ui
            .memory_mut(|m| m.data.remove_temp(code_id))
            .unwrap_or_default();

        // If the input hash has changed, reset the working code string.
        let hash = branch_hash(branch);
        if hash != state.branch_hash {
            state.branch_hash = hash;
            state.code = branch.src().to_string();
        }

        let language = "scm";
        let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());

        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut layout_job = egui_extras::syntax_highlighting::highlight(
                ui.ctx(),
                ui.style(),
                &theme,
                buf.as_str(),
                language,
            );
            layout_job.wrap.max_width = wrap_width;
            ui.fonts_mut(|fonts| fonts.layout_job(layout_job))
        };

        // Find the longest line width.
        let mut max_line_width: f32 = 0.0;
        let font_sel = egui::FontSelection::from(egui::TextStyle::Monospace);
        let font_id = font_sel.resolve(ui.style());
        ui.fonts_mut(|fonts| {
            for line in state.code.split('\n').clone() {
                let galley = fonts.layout_no_wrap(
                    line.to_string(),
                    font_id.clone(),
                    egui::Color32::PLACEHOLDER,
                );
                max_line_width = max_line_width.max(galley.rect.width());
            }
        });
        max_line_width += 7.0;

        let response = ui.add(
            egui::TextEdit::multiline(&mut state.code)
                .id(id)
                .code_editor()
                .font(font_id)
                .desired_rows(1)
                .desired_width(max_line_width)
                .hint_text("(if (= 0 $x) (list 0 '()) (list 1 '()))")
                .layouter(&mut layouter),
        );
        if response.changed() {
            if let Ok(new_branch) =
                gantz_core::node::Branch::new(&state.code, branch.branch_conns().to_vec())
            {
                *branch = new_branch;
            }
        }

        // Persist the WIP editing code.
        ui.memory_mut(|m| m.data.insert_temp(code_id, state));

        response
    }
}

impl NodeUi for gantz_core::node::Branch {
    fn name(&self, _: &dyn crate::Registry) -> &str {
        "branch"
    }

    fn ui(
        &mut self,
        ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| {
            let id = egui::Id::new("BranchEdit").with(ctx.path());
            ui.add(BranchEdit::new(self, id))
        })
    }

    fn inspector_rows(&mut self, _ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
        let row_h = crate::widget::node_inspector::table_row_h(body.ui_mut());

        // Outputs count.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.add(egui::Label::new("outputs").selectable(false))
                    .on_hover_text("the number of outputs");
            });
            row.col(|ui| {
                let mut n = self.outputs() as i32;
                if ui
                    .add(egui::DragValue::new(&mut n).range(1..=16).speed(0.1))
                    .on_hover_text("the number of outputs")
                    .changed()
                {
                    self.set_outputs(n.clamp(1, 16) as u8);
                    // Ensure all branches still have at least 2 entries.
                    ensure_min_branches(self);
                }
            });
        });

        // Branch count.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.add(egui::Label::new("branches").selectable(false))
                    .on_hover_text("the number of possible branches");
            });
            row.col(|ui| {
                let mut n = self.n_branches() as i32;
                if ui
                    .add(egui::DragValue::new(&mut n).range(1..=16).speed(0.1))
                    .on_hover_text("the number of possible branches")
                    .changed()
                {
                    resize_branches(self, n.clamp(1, 16) as usize);
                }
            });
        });

        // Checkbox grid: rows = branches, columns = outputs.
        let outputs = self.outputs() as usize;
        let n_branches = self.n_branches();
        let mut branches = self.branch_conns().to_vec();
        let mut changed = false;

        for branch_ix in 0..n_branches {
            body.row(row_h, |mut row| {
                row.col(|ui| {
                    ui.add(egui::Label::new(format!("branch {branch_ix}")).selectable(false))
                        .on_hover_ui(|ui| {
                            ui.label(format!(
                                "the active outputs for the branch at index {branch_ix}"
                            ));
                        });
                });
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        ui.style_mut().spacing.item_spacing.x *= 0.25;
                        for out_ix in 0..outputs {
                            let mut active = branches[branch_ix].get(out_ix).unwrap_or(false);
                            if ui
                                .checkbox(&mut active, "")
                                .on_hover_ui(|ui| {
                                    ui.label(format!("output {out_ix}"));
                                })
                                .changed()
                            {
                                branches[branch_ix].set(out_ix, active).ok();
                                changed = true;
                            }
                        }
                    });
                });
            });
        }

        if changed {
            self.set_branch_conns(branches);
        }
    }
}

/// Ensure at least 1 branch exists after an output resize.
fn ensure_min_branches(branch: &mut gantz_core::node::Branch) {
    if branch.n_branches() < 1 {
        let out_len = branch.outputs() as usize;
        let branches =
            vec![gantz_core::node::Conns::unconnected(out_len).expect("out_len in range")];
        branch.set_branch_conns(branches);
    }
}

/// Resize the branch count, adding unconnected entries or truncating.
fn resize_branches(branch: &mut gantz_core::node::Branch, new_count: usize) {
    let out_len = branch.outputs() as usize;
    let mut branches = branch.branch_conns().to_vec();
    branches.truncate(new_count);
    while branches.len() < new_count {
        branches.push(gantz_core::node::Conns::unconnected(out_len).expect("out_len in range"));
    }
    branch.set_branch_conns(branches);
}

impl<'a> BranchEdit<'a> {
    pub fn new(branch: &'a mut gantz_core::node::Branch, id: egui::Id) -> Self {
        Self { branch, id }
    }
}

fn branch_hash(branch: &gantz_core::node::Branch) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::default();
    branch.hash(&mut hasher);
    hasher.finish()
}
