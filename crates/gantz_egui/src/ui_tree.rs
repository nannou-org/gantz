//! A data-driven egui renderer for [`gantz_ui`] element trees.
//!
//! [`UiTree`] walks a decoded [`Element`] tree each frame, resolving widget
//! bindings against VM state and reporting interactions as response
//! payloads. It is the single rendering path for widget node GUIs: builtins
//! construct their own fragments and render through it, and graph-defined
//! GUIs evaluate to trees rendered the same way.
//!
//! ## Bindings
//!
//! A widget's `bind` path is relative to the graph its tree was defined in.
//! The walker resolves it as `instance prefix ++ scope prefixes ++ bind
//! path` and reads via [`NodeCtx::extract_value_at`], writes via
//! [`NodeCtx::update_value_at`]. Those touch VM runtime state only, so
//! interpreting a tree never mutates CA-affecting state and
//! [`UiTreeResponse`] carries no `changed` flag.
//!
//! ## Identity
//!
//! Every element's [`egui::Id`] derives from the render root's id, its
//! structural position beneath its parent (overridden by a `key` attr), and
//! any enclosing `scope` ids. Label text never contributes, so relabelling
//! never resets widget state.
//!
//! ## Events
//!
//! Controls write their bound state on real user edits only (an egui
//! `changed` signal guarded by a value comparison), then queue a push
//! evaluation at the bound node when their `push` attr is on (`button`
//! always pushes). Restoring or externally rewriting state never emits.
//! Push evaluations name the compiled entry fn from the bound node's output
//! count, so callers provide a resolver via [`UiTree::n_outputs`].
//!
//! ## Embeds
//!
//! A `ref-gui` element embeds a referenced graph instance's own GUI. The
//! walker resolves the chain of ref ids entered so far to the leaf graph's
//! body marker via the [`UiTree::ref_gui`] resolver, reads the marker's
//! stored tree from VM state, decodes it, and renders it inside a group
//! frame. The ref id becomes an implicit scope for the subtree, so the
//! embedded tree's binds resolve inside the referenced instance.
//!
//! ## Errors
//!
//! [`gantz_ui::decode()`] is total: every node of the tree is renderable and
//! [`Element::Error`] is the only inline-error case, drawn as an error chip
//! in place while siblings render normally. Decode warnings never reach the
//! walker for the tree it is handed. Callers that decode (rather than
//! construct) trees surface them themselves; warnings from embedded marker
//! trees are dropped (inspect the referenced graph to see them).

use crate::{NodeCtx, node, response::DynResponse};
use gantz_ui::{Align, BindPath, Button, Dialer, Element, Key, Matrix, Rgba, Toggle};
use steel::{SteelVal, Vector, gc::Gc};

pub(crate) mod plot;

/// The outcome of interpreting a UI tree for one frame.
///
/// Interpreting a tree never edits CA-affecting state (controls write VM
/// runtime state and queue evaluations only), so there is no `changed` flag.
#[derive(Debug, Default)]
pub struct UiTreeResponse {
    /// The union of every rendered widget's response, if anything rendered.
    pub inner: Option<egui::Response>,
    /// Payloads emitted for the application to handle after the GUI pass
    /// ([`EvalEntry`][crate::EvalEntry] push evaluations in v1).
    pub payloads: Vec<DynResponse>,
}

/// Renders a [`gantz_ui::Element`] tree, resolving bindings against VM state
/// through the given [`NodeCtx`].
///
/// This is the only seam through which the tree touches the VM: state access
/// goes via [`NodeCtx`], and everything else is reported on the returned
/// [`UiTreeResponse`].
pub struct UiTree<'a> {
    root_id: egui::Id,
    instance_prefix: &'a [node::Id],
    n_outputs: Option<&'a dyn Fn(&[node::Id]) -> Option<usize>>,
    ref_gui: Option<&'a dyn Fn(&[node::Id]) -> Option<node::Id>>,
}

/// The maximum depth of nested `ref-gui` embeds rendered before falling back
/// to an inert chip.
///
/// Registry cycle-prevention makes truly cyclic references impossible; this
/// bounds the walk's cost on corrupt data.
const MAX_REF_GUI_DEPTH: usize = 8;

/// Walker state threaded through one tree traversal.
struct Walk<'a> {
    instance_prefix: &'a [node::Id],
    /// Accumulated `(scope id ...)` prefixes enclosing the current element.
    scopes: Vec<node::Id>,
    /// The chain of `ref-gui` ids entered so far, render root outward.
    ///
    /// Kept apart from `scopes` (which extend binding paths regardless of
    /// origin): the chain is what the resolver folds over the registry.
    ref_chain: Vec<node::Id>,
    n_outputs: Option<&'a dyn Fn(&[node::Id]) -> Option<usize>>,
    ref_gui: Option<&'a dyn Fn(&[node::Id]) -> Option<node::Id>>,
    resp: UiTreeResponse,
}

impl<'a> UiTree<'a> {
    /// Interpret a tree whose widget identities derive from `root_id`.
    pub fn new(root_id: egui::Id) -> Self {
        Self {
            root_id,
            instance_prefix: &[],
            n_outputs: None,
            ref_gui: None,
        }
    }

    /// The path prefix prepended to every binding, i.e. the path at which
    /// the tree's defining graph is instanced. Empty by default.
    pub fn instance_prefix(mut self, prefix: &'a [node::Id]) -> Self {
        self.instance_prefix = prefix;
        self
    }

    /// The output count of the node at the given resolved path, required to
    /// queue push evaluations (the entry fn's identity covers the count).
    ///
    /// A node rendering its own fragment knows its own count. A caller whose
    /// tree binds arbitrary nodes resolves counts from the graph. Without a
    /// resolver (or when it returns `None`) pushes are skipped with a
    /// warning.
    pub fn n_outputs(mut self, f: &'a dyn Fn(&[node::Id]) -> Option<usize>) -> Self {
        self.n_outputs = Some(f);
        self
    }

    /// Resolve an embedded reference GUI (`ref-gui`) to its body marker.
    ///
    /// The argument is the chain of ref ids entered so far (render root
    /// outward, including the element being resolved); the return is the
    /// body marker's node id within the leaf referenced graph (see
    /// [`crate::reg::resolve_ref_chain`]). The walker reads the marker's
    /// stored tree at the resolved instance path, decodes it, and renders it
    /// inside a group frame under an implicit scope. Without a resolver (or
    /// when it returns `None`) embeds render as an inert chip.
    pub fn ref_gui(mut self, f: &'a dyn Fn(&[node::Id]) -> Option<node::Id>) -> Self {
        self.ref_gui = Some(f);
        self
    }

    /// Render the tree, resolving bindings via `ctx`.
    pub fn show(self, tree: &Element, ctx: &mut NodeCtx, ui: &mut egui::Ui) -> UiTreeResponse {
        let mut walk = Walk {
            instance_prefix: self.instance_prefix,
            scopes: Vec::new(),
            ref_chain: Vec::new(),
            n_outputs: self.n_outputs,
            ref_gui: self.ref_gui,
            resp: UiTreeResponse::default(),
        };
        walk.element(tree, self.root_id, ctx, ui);
        walk.resp
    }
}

impl Walk<'_> {
    /// Render one element with the given derived id.
    fn element(&mut self, e: &Element, id: egui::Id, ctx: &mut NodeCtx, ui: &mut egui::Ui) {
        match e {
            Element::Col(col) => {
                let cross = cross_align(col.align.unwrap_or(Align::Start));
                ui.with_layout(egui::Layout::top_down(cross), |ui| {
                    if let Some(gap) = col.gap {
                        ui.spacing_mut().item_spacing.y = gap;
                    }
                    self.children(&col.children, id, ctx, ui);
                });
            }
            Element::Row(row) => {
                // Mirror `Ui::horizontal`: seed the row with the interact
                // height so cross-alignment centres within one row, not
                // within all remaining vertical space.
                let cross = cross_align(row.align.unwrap_or(Align::Center));
                let initial = egui::vec2(
                    ui.available_size_before_wrap().x,
                    ui.spacing().interact_size.y,
                );
                ui.allocate_ui_with_layout(initial, egui::Layout::left_to_right(cross), |ui| {
                    if let Some(gap) = row.gap {
                        ui.spacing_mut().item_spacing.x = gap;
                    }
                    self.children(&row.children, id, ctx, ui);
                });
            }
            Element::Grid(grid) => {
                let cols = (grid.cols.max(1)) as usize;
                let mut g = egui::Grid::new(id).num_columns(cols);
                if let Some(gap) = grid.gap {
                    g = g.spacing([gap, gap]);
                }
                g.show(ui, |ui| {
                    for (ix, child) in grid.children.iter().enumerate() {
                        self.element(child, child_id(id, ix, child.key()), ctx, ui);
                        if (ix + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                });
            }
            Element::Frame(frame) => {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.vertical(|ui| {
                        if let Some(title) = &frame.title {
                            ui.strong(title);
                        }
                        self.children(&frame.children, id, ctx, ui);
                    });
                });
            }
            Element::Sep(_) => {
                let r = ui.separator();
                self.merge(r);
            }
            Element::Space(space) => {
                let amount = space.amount.unwrap_or_else(|| {
                    let spacing = ui.spacing().item_spacing;
                    if ui.layout().main_dir().is_horizontal() {
                        spacing.x
                    } else {
                        spacing.y
                    }
                });
                ui.add_space(amount);
            }
            Element::Scope(scope) => {
                // No visuals: children render inline under the extended
                // binding prefix, with the scope id salting their identity.
                self.scopes.push(scope.id);
                let parent = id.with(("scope", scope.id));
                self.children(&scope.children, parent, ctx, ui);
                self.scopes.pop();
            }
            Element::Dialer(dialer) => self.dialer(dialer, id, ctx, ui),
            Element::Toggle(toggle) => self.toggle(toggle, id, ctx, ui),
            Element::Button(button) => self.button(button, id, ui),
            Element::Matrix(matrix) => self.matrix(matrix, id, ctx, ui),
            Element::Label(label) => {
                let mut text = egui::RichText::new(&label.text);
                if let Some(size) = label.size {
                    text = text.size(size);
                }
                if let Some(Rgba([r, g, b, a])) = label.color {
                    text = text.color(egui::Color32::from_rgba_unmultiplied(r, g, b, a));
                }
                // Non-selectable so surrounding node bodies stay draggable.
                let r = ui.add(egui::Label::new(text).selectable(false));
                self.merge(r);
            }
            Element::Value(value) => {
                let text = match &value.bind {
                    None => "∅".to_string(),
                    Some(bind) => match ctx.extract_value_at(&self.resolve(bind)) {
                        Ok(Some(val)) => format!("{val:?}"),
                        Ok(None) => "∅".to_string(),
                        Err(_) => "ERR".to_string(),
                    },
                };
                let mut label = egui::Label::new(text).selectable(false);
                label = if value.wrap {
                    label.wrap()
                } else {
                    label.extend()
                };
                let r = ui.add(label);
                self.merge(r);
            }
            Element::Plot(plot) => self.plot(plot, id, ctx, ui),
            Element::RefGui(ref_gui) => self.ref_gui_elem(ref_gui, id, ctx, ui),
            Element::Error(err) => {
                let color = ui.visuals().error_fg_color;
                let r = egui::Frame::group(ui.style())
                    .stroke(egui::Stroke::new(1.0, color))
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(err.reason.to_string()).weak())
                                .selectable(false),
                        )
                    })
                    .inner
                    .on_hover_text(format!("tree path: {:?}", err.path.0));
                self.merge(r);
            }
        }
    }

    /// Render container children, deriving each child's id from `parent`.
    fn children(
        &mut self,
        children: &[Element],
        parent: egui::Id,
        ctx: &mut NodeCtx,
        ui: &mut egui::Ui,
    ) {
        for (ix, child) in children.iter().enumerate() {
            self.element(child, child_id(parent, ix, child.key()), ctx, ui);
        }
    }

    /// Render a numeric drag value bound to number state.
    fn dialer(&mut self, d: &Dialer, id: egui::Id, ctx: &mut NodeCtx, ui: &mut egui::Ui) {
        let Some(bind) = &d.bind else {
            let r = unbound_chip(ui, "dialer");
            self.merge(r);
            return;
        };
        let path = self.resolve(bind);
        let Ok(Some(mut val)) = ctx.extract_value_at(&path) else {
            let r = err_label(ui, "no state at the bound path");
            self.merge(r);
            return;
        };
        let original = val.clone();
        let r = ui
            .push_id(id, |ui| {
                if !d.push {
                    // Flatten the dialer's fill: a cue that editing won't
                    // fire downstream.
                    let widgets = &mut ui.visuals_mut().widgets;
                    widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                    widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                }
                let drag = |ui: &mut egui::Ui, val: &mut SteelVal| match val {
                    SteelVal::NumV(f) => {
                        let mut dv = egui::DragValue::new(f);
                        if d.min.is_some() || d.max.is_some() {
                            let (lo, hi) = (
                                d.min.unwrap_or(f64::NEG_INFINITY),
                                d.max.unwrap_or(f64::INFINITY),
                            );
                            dv = dv.range(lo..=hi);
                        }
                        if let Some(p) = d.precision {
                            dv = dv.max_decimals(p as usize);
                        }
                        if let Some(s) = d.step {
                            dv = dv.speed(s);
                        }
                        ui.add(dv)
                    }
                    SteelVal::IntV(i) => {
                        let mut dv = egui::DragValue::new(i);
                        if d.min.is_some() || d.max.is_some() {
                            let lo = d.min.map_or(isize::MIN, |m| m as isize);
                            let hi = d.max.map_or(isize::MAX, |m| m as isize);
                            dv = dv.range(lo..=hi);
                        }
                        if let Some(s) = d.step {
                            dv = dv.speed(s);
                        }
                        ui.add(dv)
                    }
                    _ => err_label(ui, "bound state is not a number"),
                };
                match &d.label {
                    Some(label) => {
                        ui.horizontal(|ui| {
                            ui.weak(label);
                            drag(ui, &mut val)
                        })
                        .inner
                    }
                    None => drag(ui, &mut val),
                }
            })
            .inner;
        if r.changed() && val != original {
            self.emit_set(ctx, &path, val, d.push);
        }
        self.merge(r);
    }

    /// Render a checkbox bound to bool state.
    fn toggle(&mut self, t: &Toggle, id: egui::Id, ctx: &mut NodeCtx, ui: &mut egui::Ui) {
        let Some(bind) = &t.bind else {
            let r = unbound_chip(ui, "toggle");
            self.merge(r);
            return;
        };
        let path = self.resolve(bind);
        let Ok(Some(val)) = ctx.extract_value_at(&path) else {
            let r = err_label(ui, "no state at the bound path");
            self.merge(r);
            return;
        };
        let SteelVal::BoolV(mut b) = val else {
            let r = err_label(ui, "bound state is not a bool");
            self.merge(r);
            return;
        };
        let text = t.label.as_deref().unwrap_or("");
        let r = ui.push_id(id, |ui| ui.checkbox(&mut b, text)).inner;
        if r.changed() {
            self.emit_set(ctx, &path, SteelVal::BoolV(b), t.push);
        }
        self.merge(r);
    }

    /// Render a button that queues a push evaluation at the bound node.
    /// Buttons never write state: bang semantics stay in the node's expr.
    fn button(&mut self, b: &Button, id: egui::Id, ui: &mut egui::Ui) {
        let text = b.label.as_deref().unwrap_or("!");
        let r = ui.push_id(id, |ui| ui.button(text)).inner;
        if r.clicked() {
            match &b.bind {
                Some(bind) => {
                    let path = self.resolve(bind);
                    self.emit_push(&path);
                }
                None => log::warn!("button with no bind pressed, nothing to push"),
            }
        }
        self.merge(r);
    }

    /// Render a grid of cells bound to rows-of-cells state (bool or number
    /// cells). Rows and columns come from the state shape, and any cell edit
    /// rebuilds and writes the whole value.
    fn matrix(&mut self, m: &Matrix, id: egui::Id, ctx: &mut NodeCtx, ui: &mut egui::Ui) {
        let Some(bind) = &m.bind else {
            let r = unbound_chip(ui, "matrix");
            self.merge(r);
            return;
        };
        let path = self.resolve(bind);
        let Ok(Some(val)) = ctx.extract_value_at(&path) else {
            let r = err_label(ui, "no state at the bound path");
            self.merge(r);
            return;
        };
        let Some(rows) = matrix_rows(&val) else {
            let r = err_label(ui, "bound state is not rows of cells");
            self.merge(r);
            return;
        };
        if rows.is_empty() {
            let r = weak_chip(ui, "∅");
            self.merge(r);
            return;
        }
        let cell_size = m.cell_size.unwrap_or(DEFAULT_CELL_SIZE);
        let mut edit: Option<(usize, usize, SteelVal)> = None;
        let r = ui
            .push_id(id, |ui| {
                ui.vertical(|ui| {
                    let mut area: Option<egui::Response> = None;
                    for (row_ix, row) in rows.iter().enumerate() {
                        ui.horizontal(|ui| {
                            for (col_ix, cell) in row.iter().enumerate() {
                                let size = [cell_size, cell_size];
                                let r = match cell {
                                    SteelVal::BoolV(b) => {
                                        let button = egui::Button::new("").selected(*b);
                                        let r = ui.add_sized(size, button);
                                        if r.clicked() {
                                            let flipped = SteelVal::BoolV(!b);
                                            edit = Some((row_ix, col_ix, flipped));
                                        }
                                        r
                                    }
                                    SteelVal::NumV(_) | SteelVal::IntV(_) => {
                                        let mut n = cell.clone();
                                        let dv = match &mut n {
                                            SteelVal::NumV(f) => egui::DragValue::new(f),
                                            SteelVal::IntV(i) => egui::DragValue::new(i),
                                            _ => unreachable!("numeric match above"),
                                        };
                                        let r = ui.add_sized(size, dv);
                                        if r.changed() && n != *cell {
                                            edit = Some((row_ix, col_ix, n));
                                        }
                                        r
                                    }
                                    _ => err_label(ui, "cell is not a bool or number"),
                                };
                                area = Some(match area.take() {
                                    Some(prev) => prev.union(r),
                                    None => r,
                                });
                            }
                        });
                    }
                    area
                })
                .inner
            })
            .inner;
        if let Some((row, col, cell)) = edit {
            if let Some(new_val) = matrix_set_cell(&val, row, col, cell) {
                self.emit_set(ctx, &path, new_val, m.push);
            }
        }
        if let Some(r) = r {
            self.merge(r);
        }
    }

    /// Render bound state through the shared plot leaf. The `mode` attr
    /// selects how the *producing* node accumulates its state and is
    /// irrelevant to drawing, so the walker ignores it.
    fn plot(&mut self, p: &gantz_ui::Plot, id: egui::Id, ctx: &mut NodeCtx, ui: &mut egui::Ui) {
        let Some(bind) = &p.bind else {
            let r = unbound_chip(ui, "plot");
            self.merge(r);
            return;
        };
        let path = self.resolve(bind);
        let channels = match ctx.extract_value_at(&path) {
            Ok(Some(val)) => plot::split_channels(&val),
            _ => Vec::new(),
        };
        let params = plot::PlotParams {
            style: p.style.unwrap_or(gantz_ui::PlotStyle::Bars),
            color: p.color.map(|Rgba(c)| c),
            grid: p.grid,
            axes: p.axes,
            interactive: p.interactive,
            y_min: p.y_min,
            y_max: p.y_max,
        };
        // Absent dimensions fill the available space (the detached view and
        // debug pane cases). Fragments with a fixed size set both.
        let size = egui::vec2(
            p.w.unwrap_or_else(|| ui.available_width()),
            p.h.unwrap_or_else(|| ui.available_height()),
        );
        let r = plot::plot_body(&params, &channels, id, size, ui);
        self.merge(r);
    }

    /// Render an embedded reference GUI: resolve the accumulated ref chain
    /// to the leaf graph's body marker, read and decode the marker's stored
    /// tree, and render it inside a group frame under an implicit scope.
    fn ref_gui_elem(
        &mut self,
        rg: &gantz_ui::RefGui,
        id: egui::Id,
        ctx: &mut NodeCtx,
        ui: &mut egui::Ui,
    ) {
        let chip = |walk: &mut Self, ui: &mut egui::Ui, why: &str| {
            let r = weak_chip(ui, &format!("ref-gui {}", rg.id)).on_hover_text(why);
            walk.merge(r);
        };
        if self.ref_chain.len() >= MAX_REF_GUI_DEPTH {
            chip(self, ui, "embed depth limit reached");
            return;
        }
        let Some(resolver) = self.ref_gui else {
            chip(self, ui, "embedded reference GUIs need a resolver");
            return;
        };
        self.ref_chain.push(rg.id);
        let decoded = match resolver(&self.ref_chain) {
            None => Err("unresolvable reference or no body marker"),
            Some(marker_id) => {
                // Resolved before the implicit scope push below: the marker
                // lives inside the instance at `rg.id`.
                let path = self.resolve(&BindPath(vec![rg.id, marker_id]));
                match ctx.extract_value_at(&path) {
                    Ok(Some(val)) if !matches!(val, SteelVal::Void) => Ok(
                        gantz_ui::codec::steel::decode(&val, &gantz_ui::Limits::default()),
                    ),
                    Ok(_) => Err("the referenced GUI has no state yet"),
                    Err(_) => Err("failed to read the referenced GUI's state"),
                }
            }
        };
        match decoded {
            Err(why) => chip(self, ui, why),
            Ok(decoded) => {
                // The ref id becomes an implicit scope: the embedded tree's
                // binds resolve inside the referenced instance. The id salt
                // is distinct from the scope arm's so a sibling
                // `(scope <id> ...)` can never collide.
                self.scopes.push(rg.id);
                let child = id.with(("ref-gui", rg.id));
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    self.element(&decoded.root, child, ctx, ui);
                });
                self.scopes.pop();
            }
        }
        self.ref_chain.pop();
    }

    /// Write an edited value to its bound state, then queue a push
    /// evaluation when `push` is on.
    fn emit_set(&mut self, ctx: &mut NodeCtx, path: &[node::Id], val: SteelVal, push: bool) {
        if let Err(e) = ctx.update_value_at(path, val) {
            log::warn!("failed to write bound state at {path:?}: {e}");
            return;
        }
        if push {
            self.emit_push(path);
        }
    }

    /// Queue a push evaluation for the node at `path`.
    fn emit_push(&mut self, path: &[node::Id]) {
        let n_outputs = self.n_outputs.and_then(|f| f(path));
        let Some(n) = n_outputs.and_then(|n| u8::try_from(n).ok()) else {
            log::warn!(
                "cannot resolve the output count of the node at {path:?}, skipping push eval"
            );
            return;
        };
        let ep = gantz_core::compile::entrypoint::push(path.to_vec(), n);
        self.resp
            .payloads
            .push(DynResponse::new(crate::EvalEntry(ep)));
    }

    /// Resolve a binding to its full state-tree path.
    fn resolve(&self, bind: &BindPath) -> Vec<node::Id> {
        resolve_path(self.instance_prefix, &self.scopes, bind)
    }

    /// Union `r` into the response.
    fn merge(&mut self, r: egui::Response) {
        self.resp.inner = Some(match self.resp.inner.take() {
            Some(prev) => prev.union(r),
            None => r,
        });
    }
}

/// A child's identity: its structural position beneath `parent`, overridden
/// by a `key` attr. Key spaces are disjoint from position spaces, so keyed
/// and unkeyed siblings can never collide.
fn child_id(parent: egui::Id, position: usize, key: Option<&Key>) -> egui::Id {
    match key {
        Some(Key::Str(s)) => parent.with(("k", s.as_str())),
        Some(Key::Int(i)) => parent.with(("k", i)),
        None => parent.with(position),
    }
}

/// The full state-tree path of a binding: the tree's instance prefix, then
/// the accumulated scope prefixes, then the bind path itself.
fn resolve_path(prefix: &[node::Id], scopes: &[node::Id], bind: &BindPath) -> Vec<node::Id> {
    prefix
        .iter()
        .chain(scopes.iter())
        .chain(bind.0.iter())
        .copied()
        .collect()
}

/// Map a cross-axis alignment to egui's.
fn cross_align(align: Align) -> egui::Align {
    match align {
        Align::Start => egui::Align::Min,
        Align::Center => egui::Align::Center,
        Align::End => egui::Align::Max,
    }
}

/// The default side length of a matrix cell.
const DEFAULT_CELL_SIZE: f32 = 18.0;

/// A matrix value's cells as rows, or `None` when the bound state is not
/// rows-of-cells shaped. Lists and vectors are accepted interchangeably.
fn matrix_rows(val: &SteelVal) -> Option<Vec<Vec<SteelVal>>> {
    let (_, rows) = seq_elems(val)?;
    rows.iter()
        .map(|row| seq_elems(row).map(|(_, cells)| cells))
        .collect()
}

/// Rebuild a whole matrix value with one cell replaced, preserving the
/// list-vs-vector shape at both levels. `None` when `val` is not a matrix or
/// the cell is out of bounds.
fn matrix_set_cell(val: &SteelVal, row: usize, col: usize, cell: SteelVal) -> Option<SteelVal> {
    let (outer, mut rows) = seq_elems(val)?;
    let (inner, mut cells) = seq_elems(rows.get(row)?)?;
    if col >= cells.len() {
        return None;
    }
    cells[col] = cell;
    rows[row] = seq_rebuild(inner, cells);
    Some(seq_rebuild(outer, rows))
}

/// Whether a value is a steel list or vector (interchangeable throughout).
#[derive(Clone, Copy)]
enum Seq {
    List,
    Vector,
}

/// A sequence value's kind and elements.
fn seq_elems(val: &SteelVal) -> Option<(Seq, Vec<SteelVal>)> {
    match val {
        SteelVal::ListV(l) => Some((Seq::List, l.iter().cloned().collect())),
        SteelVal::VectorV(v) => Some((Seq::Vector, v.iter().cloned().collect())),
        _ => None,
    }
}

/// Rebuild a sequence of the given kind.
fn seq_rebuild(seq: Seq, items: Vec<SteelVal>) -> SteelVal {
    match seq {
        Seq::List => SteelVal::ListV(items.into_iter().collect()),
        Seq::Vector => {
            let v: Vector<SteelVal> = items.into_iter().collect();
            SteelVal::VectorV(Gc::new(v).into())
        }
    }
}

/// An inline chip for a control missing its `bind` attr.
fn unbound_chip(ui: &mut egui::Ui, tag: &str) -> egui::Response {
    weak_chip(ui, tag).on_hover_text("no `bind` attribute")
}

/// An inline "ERR" label for a control whose bound state has the wrong
/// shape, with the reason on hover.
fn err_label(ui: &mut egui::Ui, why: &str) -> egui::Response {
    ui.add(egui::Label::new("ERR").selectable(false))
        .on_hover_text(why)
}

/// A small framed chip with weak text.
fn weak_chip(ui: &mut egui::Ui, text: &str) -> egui::Response {
    egui::Frame::group(ui.style())
        .show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(text).weak()).selectable(false))
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_path_concatenates_prefix_scopes_and_bind() {
        let bind = BindPath(vec![7]);
        assert_eq!(resolve_path(&[], &[], &bind), vec![7]);
        assert_eq!(resolve_path(&[1], &[], &bind), vec![1, 7]);
        assert_eq!(resolve_path(&[], &[4], &bind), vec![4, 7]);
        assert_eq!(resolve_path(&[1, 2], &[3, 4], &bind), vec![1, 2, 3, 4, 7]);
        let deep = BindPath(vec![5, 6]);
        assert_eq!(resolve_path(&[1], &[2], &deep), vec![1, 2, 5, 6]);
    }

    #[test]
    fn keyed_children_keep_their_id_under_reorder() {
        let parent = egui::Id::new("parent");
        let key = Key::Str("a".to_string());
        assert_eq!(
            child_id(parent, 0, Some(&key)),
            child_id(parent, 5, Some(&key)),
        );
        assert_eq!(
            child_id(parent, 0, Some(&Key::Int(3))),
            child_id(parent, 9, Some(&Key::Int(3))),
        );
    }

    #[test]
    fn unkeyed_children_are_identified_by_position() {
        let parent = egui::Id::new("parent");
        assert_ne!(child_id(parent, 0, None), child_id(parent, 1, None));
        assert_eq!(child_id(parent, 2, None), child_id(parent, 2, None));
    }

    #[test]
    fn distinct_keys_are_distinct_ids() {
        let parent = egui::Id::new("parent");
        let a = Key::Str("a".to_string());
        let b = Key::Str("b".to_string());
        assert_ne!(child_id(parent, 0, Some(&a)), child_id(parent, 0, Some(&b)));
        assert_ne!(
            child_id(parent, 0, Some(&Key::Int(1))),
            child_id(parent, 0, Some(&Key::Int(2))),
        );
    }

    #[test]
    fn ids_derive_from_the_parent() {
        let (p1, p2) = (egui::Id::new("p1"), egui::Id::new("p2"));
        assert_ne!(child_id(p1, 0, None), child_id(p2, 0, None));
        let key = Key::Str("a".to_string());
        assert_ne!(child_id(p1, 0, Some(&key)), child_id(p2, 0, Some(&key)));
    }

    fn list(items: Vec<SteelVal>) -> SteelVal {
        SteelVal::ListV(items.into_iter().collect())
    }

    fn vector(items: Vec<SteelVal>) -> SteelVal {
        let v: Vector<SteelVal> = items.into_iter().collect();
        SteelVal::VectorV(Gc::new(v).into())
    }

    #[test]
    fn matrix_rows_requires_rows_of_cells() {
        let m = list(vec![list(vec![
            SteelVal::BoolV(true),
            SteelVal::BoolV(false),
        ])]);
        let rows = matrix_rows(&m).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 2);

        // A flat sequence of numbers is not a matrix.
        let flat = list(vec![SteelVal::IntV(1), SteelVal::IntV(2)]);
        assert_eq!(matrix_rows(&flat), None);
        // Nor is a lone number.
        assert_eq!(matrix_rows(&SteelVal::IntV(1)), None);
    }

    #[test]
    fn matrix_set_cell_replaces_one_cell() {
        let m = list(vec![
            list(vec![SteelVal::BoolV(false), SteelVal::BoolV(false)]),
            list(vec![SteelVal::BoolV(false), SteelVal::BoolV(false)]),
        ]);
        let new = matrix_set_cell(&m, 1, 0, SteelVal::BoolV(true)).unwrap();
        let rows = matrix_rows(&new).unwrap();
        assert_eq!(
            rows[0],
            vec![SteelVal::BoolV(false), SteelVal::BoolV(false)]
        );
        assert_eq!(rows[1], vec![SteelVal::BoolV(true), SteelVal::BoolV(false)]);
    }

    #[test]
    fn matrix_set_cell_preserves_list_vs_vector_shape() {
        // A vector of lists keeps its shape at both levels.
        let m = vector(vec![
            list(vec![SteelVal::IntV(0), SteelVal::IntV(1)]),
            list(vec![SteelVal::IntV(2), SteelVal::IntV(3)]),
        ]);
        let new = matrix_set_cell(&m, 0, 1, SteelVal::IntV(9)).unwrap();
        let SteelVal::VectorV(rows) = &new else {
            panic!("outer vector became {new:?}");
        };
        let row0 = rows.iter().next().unwrap();
        assert!(
            matches!(row0, SteelVal::ListV(_)),
            "inner list became {row0:?}"
        );
        let rows = matrix_rows(&new).unwrap();
        assert_eq!(rows[0], vec![SteelVal::IntV(0), SteelVal::IntV(9)]);
        assert_eq!(rows[1], vec![SteelVal::IntV(2), SteelVal::IntV(3)]);
    }

    /// A test VM with an initialised state tree and empty environment,
    /// running `f` with a walker-ready `NodeCtx` inside a test `Ui`.
    fn run_with_ctx(
        state: Vec<(Vec<node::Id>, SteelVal)>,
        f: impl FnMut(&mut NodeCtx, &mut egui::Ui),
    ) {
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
        vm.register_value(gantz_core::ROOT_STATE, SteelVal::empty_hashmap());
        for (path, val) in state {
            gantz_core::node::state::update_value(&mut vm, &path, val).unwrap();
        }
        let mut f = f;
        egui::__run_test_ui(|ui| {
            let mut writes = Vec::new();
            let mut ctx = crate::NodeCtx::new(&env, &[][..], &[], &[], &[], &mut vm, &mut writes);
            f(&mut ctx, ui);
        });
    }

    /// The encoded marker tree for an embed containing another embed.
    fn ref_gui_elem(id: node::Id) -> SteelVal {
        let elem = Element::RefGui(gantz_ui::RefGui { id, key: None });
        gantz_ui::codec::steel::encode(&elem)
    }

    #[test]
    fn ref_gui_without_resolver_renders_chip() {
        let tree = Element::RefGui(gantz_ui::RefGui { id: 3, key: None });
        run_with_ctx(vec![], |ctx, ui| {
            let r = UiTree::new(egui::Id::new("t")).show(&tree, ctx, ui);
            assert!(r.inner.is_some(), "the fallback chip must render");
            assert!(r.payloads.is_empty());
        });
    }

    /// Each nested embed extends the chain handed to the resolver: the
    /// walker resolves `[3]` for the root embed, then `[3, 5]` for the embed
    /// inside its marker tree.
    #[test]
    fn ref_gui_resolver_receives_full_chain() {
        let tree = Element::RefGui(gantz_ui::RefGui { id: 3, key: None });
        let leaf = gantz_ui::codec::steel::encode(&Element::Sep(gantz_ui::Sep { key: None }));
        let state = vec![(vec![3, 9], ref_gui_elem(5)), (vec![3, 5, 9], leaf)];
        let calls = std::cell::RefCell::new(Vec::new());
        let resolver = |chain: &[node::Id]| {
            calls.borrow_mut().push(chain.to_vec());
            Some(9)
        };
        run_with_ctx(state, |ctx, ui| {
            UiTree::new(egui::Id::new("t"))
                .ref_gui(&resolver)
                .show(&tree, ctx, ui);
        });
        assert_eq!(calls.into_inner(), vec![vec![3], vec![3, 5]]);
    }

    /// Self-referential marker trees stop resolving at the depth limit.
    #[test]
    fn ref_gui_depth_guard_stops_at_limit() {
        let tree = Element::RefGui(gantz_ui::RefGui { id: 3, key: None });
        // The marker at every depth embeds `(ref-gui 3)` again.
        let state = (1..=MAX_REF_GUI_DEPTH + 1)
            .map(|depth| {
                let mut path = vec![3; depth];
                path.push(9);
                (path, ref_gui_elem(3))
            })
            .collect();
        let calls = std::cell::Cell::new(0);
        let resolver = |_: &[node::Id]| {
            calls.set(calls.get() + 1);
            Some(9)
        };
        run_with_ctx(state, |ctx, ui| {
            UiTree::new(egui::Id::new("t"))
                .ref_gui(&resolver)
                .show(&tree, ctx, ui);
        });
        assert_eq!(calls.get(), MAX_REF_GUI_DEPTH);
    }

    #[test]
    fn matrix_set_cell_out_of_bounds_is_none() {
        let m = list(vec![list(vec![SteelVal::BoolV(false)])]);
        assert_eq!(matrix_set_cell(&m, 1, 0, SteelVal::BoolV(true)), None);
        assert_eq!(matrix_set_cell(&m, 0, 1, SteelVal::BoolV(true)), None);
        assert_eq!(
            matrix_set_cell(&SteelVal::IntV(1), 0, 0, SteelVal::BoolV(true)),
            None,
        );
    }
}
