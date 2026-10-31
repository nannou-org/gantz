//! A data-driven egui renderer for [`gantz_ui`] element trees.
//!
//! [`UiTree`] walks a decoded [`Element`] tree each frame. It resolves
//! widget bindings against VM state and reports interactions as response
//! payloads. It is the single rendering path for widget node GUIs. Builtins
//! construct their own fragments and render through it. Graph-defined GUIs
//! evaluate to trees rendered the same way. The tree model, binding
//! resolution, identity rules and decode totality are documented in
//! [`gantz_ui`].
//!
//! Bindings read and write node state through a [`TreeState`]. A node's
//! view uses its [`NodeCtx`], whose state is VM runtime state only, so
//! interpreting a tree never mutates CA-affecting state. A tree with no node,
//! such as a domain's settings tab, renders over [`Detached`].
//!
//! A control bound to a `field` reads the fields of the node that owns the
//! tree, given by [`UiTree::fields`]. Its edits come back in
//! [`UiTreeResponse::field_edits`] for the node to apply.
//!
//! Each element's [`egui::Id`] derives from the render root's id, its
//! structural position or `key` beneath its parent, and any enclosing
//! `scope` ids.
//!
//! Controls write their bound state on real user edits only. The egui
//! `changed` signal is guarded by a value comparison. When the `push` attr
//! is on, a write also queues a push evaluation at the bound node. `button`
//! always pushes. Push evaluations name the compiled entry fn from the bound
//! node's output count, so callers provide a resolver via
//! [`UiTree::n_outputs`].
//!
//! A `ref-gui` element embeds a referenced graph instance's own GUI. The
//! walker resolves the chain of ref ids entered so far to the leaf graph's
//! body marker via the [`UiTree::ref_gui`] resolver. It reads the marker's
//! stored tree from VM state, decodes it, and renders it inside a group
//! frame under an implicit scope.
//!
//! [`Element::Error`] draws as an error chip in place while siblings render
//! normally. Callers that decode trees surface the decode warnings
//! themselves. Warnings from embedded marker trees are dropped. Inspect the
//! referenced graph to see them.

use crate::{NodeCtx, node, response::DynResponse};
use gantz_core::datum::{self, Datum};
use gantz_ui::{
    Align, At, BindPath, Bound, Button, Choice, Color, Dialer, Edit, Element, EnvelopeElem, Field,
    Key, Matrix, Rgba, TextInput, Toggle, Tone,
};
use steel::{HashMap, SteelErr, SteelVal, Vector, gc::Gc};

pub mod canvas;
pub mod envelope;
pub mod plot;

/// The node state that a tree's bound controls read and write. See the
/// module docs.
pub trait TreeState {
    /// The state at `path`, if any.
    fn value_at(&self, path: &[node::Id]) -> Result<Option<SteelVal>, SteelErr>;

    /// Write `val` as the state at `path`.
    fn set_value_at(&mut self, path: &[node::Id], val: SteelVal) -> Result<(), SteelErr>;
}

/// No node state, for a tree that no node owns. A bound control shows that
/// its path holds nothing, and its writes fail.
#[derive(Clone, Copy, Debug, Default)]
pub struct Detached;

/// The outcome of interpreting a UI tree for one frame.
///
/// Controls write VM runtime state and queue evaluations only. Interpreting
/// a tree never edits CA-affecting state, so there is no `changed` flag.
/// Edits to the fields of the node that owns the tree are reported for the
/// node to apply, see [`UiTree::fields`].
#[derive(Debug, Default)]
pub struct UiTreeResponse {
    /// The union of every rendered widget's response, if anything rendered.
    pub inner: Option<egui::Response>,
    /// Payloads for the application to handle after the GUI pass. In v1
    /// these are [`crate::EvalEntry`] push evaluations.
    pub payloads: Vec<DynResponse>,
    /// Edits to fields of the node that owns the tree, in the order made.
    pub field_edits: Vec<(Field, Datum)>,
    /// Edits for the node that owns the tree to apply, in the order made, with
    /// the control's new value. A button sends `Datum::Null`.
    pub edits: Vec<(Edit, Datum)>,
    /// Edits of a gesture in progress, such as each step of a drag, with the
    /// value so far. The gesture's end comes as one of [`Self::edits`]. A
    /// node can preview a live edit without a change of the node.
    pub live_edits: Vec<(Edit, Datum)>,
    /// Requests to import a file into the node that owns the tree, each with
    /// its edit and the name of its importer. See [`gantz_ui::File`].
    pub imports: Vec<(Edit, String)>,
}

/// Where a control reads and writes its value.
enum Target {
    /// VM state at a resolved path, or at keys within it.
    State(Vec<node::Id>, Option<At>),
    /// A field of the node that owns the tree.
    Field(Field),
    /// An edit that the node that owns the tree applies.
    Edit(Edit),
}

/// What a control binds. See [`Walk::target_value`].
struct Binding<'b> {
    bind: &'b Option<BindPath>,
    at: &'b Option<At>,
    field: &'b Option<Field>,
    on: &'b Option<Edit>,
    /// The value that a control with an edit shows.
    value: Option<SteelVal>,
}

/// The buffered text of a text input, held in egui temp memory, so an edit
/// commits once it settles rather than per keystroke.
#[derive(Clone, Default)]
struct TextBuffer {
    /// The text being edited.
    text: String,
    /// The bound text when the buffer last synced with it.
    source: String,
    /// The time of the last keystroke.
    last_edit: f64,
}

/// Seconds of no typing before a text input commits a changed buffer.
const TEXT_FLUSH: f64 = 2.0;
/// The tip of a colour control's button that clears its colour.
const THEME_TIP: &str = "follow the theme's colour";

/// Renders a [`gantz_ui::Element`] tree and resolves bindings against VM
/// state through the given [`NodeCtx`].
///
/// This is the only seam through which the tree touches the VM. State access
/// goes via [`NodeCtx`]. Everything else is reported on the returned
/// [`UiTreeResponse`].
pub struct UiTree<'a> {
    root_id: egui::Id,
    instance_prefix: &'a [node::Id],
    n_outputs: Option<&'a dyn Fn(&[node::Id]) -> Option<usize>>,
    ref_gui: Option<&'a dyn Fn(&[node::Id]) -> Option<node::Id>>,
    fields: Option<&'a Datum>,
    owned: bool,
}

/// The maximum depth of nested `ref-gui` embeds rendered before falling back
/// to an inert chip.
///
/// Registry cycle-prevention makes truly cyclic references impossible. This
/// bounds the walk's cost on corrupt data.
const MAX_REF_GUI_DEPTH: usize = 8;

/// The drag, in points, that takes a dialer bounded on both sides across its
/// whole range, when it sets no step of its own.
pub(crate) const RANGE_DRAG: f64 = 200.0;

/// The most decimals a bounded dialer shows when it sets no precision.
const MAX_DECIMALS: usize = 6;

/// Walker state threaded through one tree traversal.
struct Walk<'a> {
    instance_prefix: &'a [node::Id],
    /// Accumulated `(scope id ...)` prefixes enclosing the current element.
    scopes: Vec<node::Id>,
    /// The chain of `ref-gui` ids entered so far, render root outward.
    ///
    /// Kept apart from `scopes`, which extend binding paths regardless of
    /// origin. The chain is what the resolver folds over the registry.
    ref_chain: Vec<node::Id>,
    n_outputs: Option<&'a dyn Fn(&[node::Id]) -> Option<usize>>,
    ref_gui: Option<&'a dyn Fn(&[node::Id]) -> Option<node::Id>>,
    fields: Option<&'a Datum>,
    /// Whether an owner takes the `on` edits and the imports of the tree.
    owned: bool,
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
            fields: None,
            owned: false,
        }
    }

    /// The fields of the node that owns the tree, the map of its stored form.
    /// Field-bound controls read them and report edits in
    /// [`UiTreeResponse::field_edits`]. Without fields, such controls are
    /// inert.
    pub fn fields(mut self, fields: &'a Datum) -> Self {
        self.fields = Some(fields);
        self
    }

    /// Mark the tree as one of a node or a domain that applies its `on`
    /// edits and imports. A tree with [`fields`](Self::fields) is one of a
    /// node too. Without an owner, such controls are inert.
    pub fn owned(mut self) -> Self {
        self.owned = true;
        self
    }

    /// The path prefix prepended to every binding. It is the path at which
    /// the tree's defining graph is instanced. Empty by default.
    pub fn instance_prefix(mut self, prefix: &'a [node::Id]) -> Self {
        self.instance_prefix = prefix;
        self
    }

    /// The output count of the node at the given resolved path. Push
    /// evaluations need it because the entry fn's identity covers the count.
    ///
    /// A node rendering its own fragment knows its own count. A caller whose
    /// tree binds arbitrary nodes resolves counts from the graph. Without a
    /// resolver, or when it returns `None`, pushes are skipped with a
    /// warning.
    pub fn n_outputs(mut self, f: &'a dyn Fn(&[node::Id]) -> Option<usize>) -> Self {
        self.n_outputs = Some(f);
        self
    }

    /// Resolve an embedded reference GUI (`ref-gui`) to its body marker.
    ///
    /// The argument is the chain of ref ids entered so far, render root
    /// outward, including the element being resolved. The return is the
    /// body marker's node id within the leaf referenced graph. See
    /// [`crate::reg::resolve_ref_chain`]. The walker reads the marker's
    /// stored tree at the resolved instance path, decodes it, and renders it
    /// inside a group frame under an implicit scope. Without a resolver, or
    /// when it returns `None`, embeds render as an inert chip.
    pub fn ref_gui(mut self, f: &'a dyn Fn(&[node::Id]) -> Option<node::Id>) -> Self {
        self.ref_gui = Some(f);
        self
    }

    /// Render the tree, resolving bindings via `ctx`.
    pub fn show(
        self,
        tree: &Element,
        ctx: &mut dyn TreeState,
        ui: &mut egui::Ui,
    ) -> UiTreeResponse {
        let mut walk = Walk {
            instance_prefix: self.instance_prefix,
            scopes: Vec::new(),
            ref_chain: Vec::new(),
            n_outputs: self.n_outputs,
            ref_gui: self.ref_gui,
            fields: self.fields,
            owned: self.owned || self.fields.is_some(),
            resp: UiTreeResponse::default(),
        };
        walk.element(tree, self.root_id, ctx, ui);
        walk.resp
    }
}

impl Walk<'_> {
    /// Render one element with the given derived id.
    fn element(&mut self, e: &Element, id: egui::Id, ctx: &mut dyn TreeState, ui: &mut egui::Ui) {
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
                // Mirror `Ui::horizontal`. Seed the row with the interact
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
                // No visuals. Children render inline under the extended
                // binding prefix, with the scope id salting their identity.
                self.scopes.push(scope.id);
                let parent = id.with(("scope", scope.id));
                self.children(&scope.children, parent, ctx, ui);
                self.scopes.pop();
            }
            Element::Dialer(dialer) => self.dialer(dialer, id, ctx, ui),
            Element::Toggle(toggle) => self.toggle(toggle, id, ctx, ui),
            Element::Bound(bound) => self.bound(bound, id, ctx, ui),
            Element::Choice(choice) => self.choice(choice, id, ctx, ui),
            Element::TextInput(input) => self.text_input(input, id, ctx, ui),
            Element::Color(color) => self.color(color, id, ctx, ui),
            Element::Button(button) => self.button(button, id, ui),
            Element::File(file) => self.file(file, id, ui),
            Element::Matrix(matrix) => self.matrix(matrix, id, ctx, ui),
            Element::Label(label) => {
                let bound = label.bind.as_ref().and_then(|bind| {
                    let state = ctx.value_at(&self.resolve(bind)).ok().flatten()?;
                    match &label.at {
                        Some(at) => state_at(&state, &at.0),
                        None => Some(state),
                    }
                });
                let shown = match bound {
                    Some(SteelVal::StringV(s)) => s.to_string(),
                    Some(SteelVal::Void) | None => label.text.clone(),
                    Some(val) => format!("{val:?}"),
                };
                let mut text = egui::RichText::new(shown);
                if let Some(size) = label.size {
                    text = text.size(size);
                }
                if label.mono {
                    text = text.monospace();
                }
                let visuals = ui.visuals();
                let color = match (label.color, label.tone) {
                    (Some(Rgba([r, g, b, a])), _) => {
                        Some(egui::Color32::from_rgba_unmultiplied(r, g, b, a))
                    }
                    (None, Some(Tone::Weak)) => Some(visuals.weak_text_color()),
                    (None, Some(Tone::Strong)) => Some(visuals.strong_text_color()),
                    (None, Some(Tone::Warn)) => Some(visuals.warn_fg_color),
                    (None, Some(Tone::Error)) => Some(visuals.error_fg_color),
                    (None, None) => None,
                };
                if let Some(color) = color {
                    text = text.color(color);
                }
                // Non-selectable so surrounding node bodies stay draggable.
                let r = ui.add(egui::Label::new(text).selectable(false));
                let r = with_tip(r, label.tip.as_deref());
                self.merge(r);
            }
            Element::Value(value) => {
                let text = match &value.bind {
                    None => "∅".to_string(),
                    Some(bind) => match ctx.value_at(&self.resolve(bind)) {
                        // A value that has not arrived yet shows as a dash.
                        Ok(Some(SteelVal::Void)) => "—".to_string(),
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
            Element::Canvas(canvas) => self.canvas(canvas, id, ctx, ui),
            Element::Envelope(e) => self.envelope(e, id, ui),
            Element::RefGui(ref_gui) => self.ref_gui_elem(ref_gui, id, ctx, ui),
            Element::Error(err) => {
                let color = ui.visuals().error_fg_color;
                let r = egui::Frame::group(ui.style())
                    .stroke(egui::Stroke::new(1.0f32, color))
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
        ctx: &mut dyn TreeState,
        ui: &mut egui::Ui,
    ) {
        for (ix, child) in children.iter().enumerate() {
            self.element(child, child_id(parent, ix, child.key()), ctx, ui);
        }
    }

    /// Render a numeric drag value bound to number state or a number field.
    fn dialer(&mut self, d: &Dialer, id: egui::Id, ctx: &mut dyn TreeState, ui: &mut egui::Ui) {
        let Some((target, mut val)) = self.target_value(
            binding(&d.bind, &d.at, &d.field, &d.on, d.value.map(SteelVal::NumV)),
            "dialer",
            ctx,
            ui,
        ) else {
            return;
        };
        let original = val.clone();
        let r = ui
            .push_id(id, |ui| {
                if !d.push {
                    // Flatten the dialer's fill as a cue that editing does
                    // not fire downstream.
                    let widgets = &mut ui.visuals_mut().widgets;
                    widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                    widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                }
                // A bounded dialer fills from the left to show where its value
                // sits in the range. The fill goes in a slot behind the
                // dialer, whose own fill is translucent.
                let drag = |ui: &mut egui::Ui, val: &mut SteelVal| {
                    let slot = ui.painter().add(egui::Shape::Noop);
                    let r = drag_value(ui, d, val);
                    if let Some(t) = range_fraction(d, val) {
                        paint_range_fill(ui, slot, r.rect, t);
                    }
                    r
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
        let r = with_tip(r, d.tip.as_deref());
        if r.changed() && val != original {
            self.emit_target(ctx, target, val, d.push);
        }
        self.merge(r);
    }

    /// Render a checkbox bound to bool state or a bool field.
    fn toggle(&mut self, t: &Toggle, id: egui::Id, ctx: &mut dyn TreeState, ui: &mut egui::Ui) {
        let Some((target, val)) = self.target_value(
            binding(
                &t.bind,
                &t.at,
                &t.field,
                &t.on,
                Some(SteelVal::BoolV(t.value)),
            ),
            "toggle",
            ctx,
            ui,
        ) else {
            return;
        };
        let SteelVal::BoolV(mut b) = val else {
            let r = err_label(ui, "bound state is not a bool");
            self.merge(r);
            return;
        };
        let text = t.label.as_deref().unwrap_or("");
        let r = ui.push_id(id, |ui| match t.pill {
            true => {
                let strong = ui.visuals().strong_text_color();
                ui.add(crate::widget::LabelToggle::new(text, &mut b).selected_color(strong))
            }
            false => ui.checkbox(&mut b, text),
        });
        let r = with_tip(claim_press(r.inner), t.tip.as_deref());
        if r.changed() {
            self.emit_target(ctx, target, SteelVal::BoolV(b), t.push);
        }
        self.merge(r);
    }

    /// Where a control reads and writes, and its value there. A field comes
    /// first, then an edit, then a bound path. Shows why when there is no
    /// value and returns `None`.
    fn target_value(
        &mut self,
        binding: Binding<'_>,
        tag: &str,
        ctx: &mut dyn TreeState,
        ui: &mut egui::Ui,
    ) -> Option<(Target, SteelVal)> {
        let Some(field) = binding.field else {
            if let Some(edit) = binding.on {
                if !self.owned {
                    let why = "an edit goes only to the node of its own view";
                    let r = weak_chip(ui, tag).on_hover_text(why);
                    self.merge(r);
                    return None;
                }
                let Some(val) = binding.value else {
                    let r = err_label(ui, "a control with an edit needs a `value`");
                    self.merge(r);
                    return None;
                };
                return Some((Target::Edit(edit.clone()), val));
            }
            let (path, state) = self.bound_state(binding.bind, tag, ctx, ui)?;
            let val = match binding.at {
                None => state,
                Some(at) => match state_at(&state, &at.0) {
                    Some(val) => val,
                    None => {
                        let r = err_label(ui, "no state at the bound keys");
                        self.merge(r);
                        return None;
                    }
                },
            };
            return Some((Target::State(path, binding.at.clone()), val));
        };
        let Some(fields) = self.fields else {
            let r = weak_chip(ui, tag).on_hover_text("a field binds only in a node's own view");
            self.merge(r);
            return None;
        };
        let Some(val) = fields.get(&field.0).map(datum::to_steel) else {
            let r = err_label(ui, "the node has no field of this name");
            self.merge(r);
            return None;
        };
        Some((Target::Field(field.clone()), val))
    }

    /// Write a control's new value to its target. See [`Walk::target_value`].
    fn emit_target(&mut self, ctx: &mut dyn TreeState, target: Target, val: SteelVal, push: bool) {
        match target {
            Target::State(path, None) => self.emit_set(ctx, &path, val, push),
            Target::State(path, Some(at)) => {
                let state = ctx.value_at(&path).ok().flatten();
                let val = with_state_at(state.as_ref(), &at.0, val);
                self.emit_set(ctx, &path, val, push)
            }
            Target::Field(field) => match datum::of_steel(&val) {
                Ok(datum) => self.resp.field_edits.push((field, datum)),
                Err(e) => log::warn!("cannot write {e} to the field `{}`", field.0),
            },
            Target::Edit(edit) => match datum::of_steel(&val) {
                Ok(datum) => self.resp.edits.push((edit, datum)),
                Err(e) => log::warn!("cannot send {e} with the edit `{}`", edit.0),
            },
        }
    }

    /// The path that `bind` resolves to and the state there. Shows why when
    /// there is none and returns `None`.
    fn bound_state(
        &mut self,
        bind: &Option<BindPath>,
        tag: &str,
        ctx: &mut dyn TreeState,
        ui: &mut egui::Ui,
    ) -> Option<(Vec<node::Id>, SteelVal)> {
        let Some(bind) = bind else {
            let r = unbound_chip(ui, tag);
            self.merge(r);
            return None;
        };
        let path = self.resolve(bind);
        let Ok(Some(val)) = ctx.value_at(&path) else {
            let r = err_label(ui, "no state at the bound path");
            self.merge(r);
            return None;
        };
        Some((path, val))
    }

    /// Render a checkbox and a number bound to number or `#f` state, or to
    /// such a field or edit, as an inspector bound row does. The checkbox sets
    /// the number or clears it.
    fn bound(&mut self, b: &Bound, id: egui::Id, ctx: &mut dyn TreeState, ui: &mut egui::Ui) {
        let shown = Some(b.value.map_or(SteelVal::BoolV(false), SteelVal::NumV));
        let binding = binding(&b.bind, &b.at, &b.field, &b.on, shown);
        let Some((target, val)) = self.target_value(binding, "bound", ctx, ui) else {
            return;
        };
        let mut bound = match val {
            SteelVal::NumV(n) => Some(n),
            SteelVal::IntV(i) => Some(i as f64),
            _ => None,
        };
        let kind = b.label.as_deref().unwrap_or("bound");
        let r = ui.push_id(id, |ui| {
            crate::widget::node_inspector::bound_col(ui, kind, &mut bound)
        });
        if r.inner {
            let val = bound.map_or(SteelVal::BoolV(false), SteelVal::NumV);
            self.emit_target(ctx, target, val, true);
        }
        self.merge(r.response);
    }

    /// Render a pill per option, bound to state or a field that holds one of
    /// them. A click sets the state or field to its option.
    fn choice(&mut self, c: &Choice, id: egui::Id, ctx: &mut dyn TreeState, ui: &mut egui::Ui) {
        let Some((target, val)) = self.target_value(
            binding(
                &c.bind,
                &c.at,
                &c.field,
                &c.on,
                c.value.clone().map(|v| SteelVal::SymbolV(v.into())),
            ),
            "choice",
            ctx,
            ui,
        ) else {
            return;
        };
        let current = match &val {
            SteelVal::SymbolV(s) | SteelVal::StringV(s) => Some(s.to_string()),
            _ => None,
        };
        let r = ui.push_id(id, |ui| {
            ui.horizontal(|ui| {
                let strong = ui.visuals().strong_text_color();
                let mut picked = None;
                for (ix, option) in c.options.iter().enumerate() {
                    let mut on = current.as_deref() == Some(option.as_str());
                    let toggle = crate::widget::LabelToggle::new(option, &mut on);
                    let r = claim_press(ui.add(toggle.selected_color(strong)));
                    let r = with_tip(r, c.tips.get(ix).map(String::as_str));
                    if r.changed() && on {
                        picked = Some(option.clone());
                    }
                }
                picked
            })
        });
        if let Some(option) = r.inner.inner {
            self.emit_target(ctx, target, SteelVal::SymbolV(option.into()), true);
        }
        self.merge(r.response);
    }

    /// Render a line of text bound to string state or a string field. The
    /// text buffers in egui temp memory and commits once the edit settles. A
    /// change to the bound text from elsewhere, such as an undo, resyncs it.
    fn text_input(
        &mut self,
        t: &TextInput,
        id: egui::Id,
        ctx: &mut dyn TreeState,
        ui: &mut egui::Ui,
    ) {
        let Some((target, val)) = self.target_value(
            binding(
                &t.bind,
                &t.at,
                &t.field,
                &t.on,
                t.value.clone().map(|v| SteelVal::StringV(v.into())),
            ),
            "text-input",
            ctx,
            ui,
        ) else {
            return;
        };
        let (SteelVal::StringV(current) | SteelVal::SymbolV(current)) = &val else {
            let r = err_label(ui, "bound state is not a string");
            self.merge(r);
            return;
        };
        let current = current.to_string();
        let mem_id = id.with("text-input");
        let mut buf: TextBuffer = ui
            .memory_mut(|m| m.data.remove_temp(mem_id))
            .unwrap_or_default();
        if buf.source != current {
            buf.source = current.clone();
            buf.text = current;
        }
        let style = match t.mono {
            true => egui::TextStyle::Monospace,
            false => egui::TextStyle::Body,
        };
        let font = egui::FontSelection::from(style).resolve(ui.style());
        // Size the input to its text, or to its hint while empty. A text edit
        // lays out within its width less its margin, so add the margin back,
        // plus 1px for sub-pixel rounding.
        let margin = egui::Margin::symmetric(4, 2);
        let measure = |ui: &egui::Ui, text: &str| {
            ui.ctx().fonts_mut(|f| {
                f.layout_no_wrap(text.to_string(), font.clone(), egui::Color32::WHITE)
                    .rect
                    .width()
            })
        };
        let hint = t.hint.as_deref().unwrap_or("");
        let width = measure(ui, &buf.text).max(measure(ui, hint)).max(16.0);
        let mut edit = egui::TextEdit::singleline(&mut buf.text)
            .font(font.clone())
            .margin(margin)
            .desired_width(width.ceil() + margin.sum().x + 1.0);
        if let Some(hint) = &t.hint {
            edit = edit.hint_text(hint);
        }
        let r = ui.push_id(id, |ui| ui.add(edit)).inner;
        let r = with_tip(r, t.tip.as_deref());
        let time = ui.input(|i| i.time);
        if r.changed() {
            buf.last_edit = time;
        }
        let dirty = buf.text != buf.source;
        let settled = time - buf.last_edit >= TEXT_FLUSH;
        if dirty && (r.lost_focus() || settled) {
            buf.source = buf.text.clone();
            let text = SteelVal::StringV(buf.text.clone().into());
            self.emit_target(ctx, target, text, t.push);
        } else if dirty {
            let wait = TEXT_FLUSH - (time - buf.last_edit);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.0)));
        }
        ui.memory_mut(|m| m.data.insert_temp(mem_id, buf));
        self.merge(r);
    }

    /// Render a colour button bound to `"#rrggbb"` or `#f` state, or to such
    /// a field or edit. `#f` shows the theme's colour. A `theme` button
    /// clears an edited colour.
    fn color(&mut self, c: &Color, id: egui::Id, ctx: &mut dyn TreeState, ui: &mut egui::Ui) {
        let shown = c.value.map(|rgba| SteelVal::StringV(rgba.hex().into()));
        let shown = shown.or(Some(SteelVal::BoolV(false)));
        let binding = binding(&c.bind, &c.at, &c.field, &c.on, shown);
        let Some((target, val)) = self.target_value(binding, "color", ctx, ui) else {
            return;
        };
        let set = match &val {
            SteelVal::StringV(s) => Rgba::parse(s),
            _ => None,
        };
        let r = ui.push_id(id, |ui| {
            ui.horizontal(|ui| {
                let mut col = plot::resolve_color(set.map(|Rgba(c)| c), ui);
                let edit = claim_press(ui.color_edit_button_srgba(&mut col));
                let edit = with_tip(edit, c.tip.as_deref());
                if edit.changed() {
                    let rgba = Rgba([col.r(), col.g(), col.b(), col.a()]);
                    return Some(SteelVal::StringV(rgba.hex().into()));
                }
                let theme = c.theme
                    && set.is_some()
                    && claim_press(ui.button("theme").on_hover_text(THEME_TIP)).clicked();
                theme.then_some(SteelVal::BoolV(false))
            })
        });
        if let Some(val) = r.inner.inner {
            self.emit_target(ctx, target, val, true);
        }
        self.merge(r.response);
    }

    /// Render a button that asks the host to import a file into the node
    /// that owns the tree.
    fn file(&mut self, f: &gantz_ui::File, id: egui::Id, ui: &mut egui::Ui) {
        let (Some(edit), Some(import)) = (&f.on, &f.import) else {
            let r = err_label(ui, "a file control needs an `on` edit and an `import`");
            self.merge(r);
            return;
        };
        if !self.owned {
            let why = "an import goes only to the node of its own view";
            let r = weak_chip(ui, "file").on_hover_text(why);
            self.merge(r);
            return;
        }
        let text = f.label.as_deref().unwrap_or("load…");
        let r = claim_press(ui.push_id(id, |ui| ui.button(text)).inner);
        let r = with_tip(r, f.tip.as_deref());
        if r.clicked() {
            self.resp.imports.push((edit.clone(), import.clone()));
        }
        self.merge(r);
    }

    /// Render a button that queues a push evaluation at the bound node.
    /// Buttons never write state. Bang semantics stay in the node's expr.
    fn button(&mut self, b: &Button, id: egui::Id, ui: &mut egui::Ui) {
        if b.on.is_some() && !self.owned {
            let why = "an edit goes only to the node of its own view";
            let r = weak_chip(ui, "button").on_hover_text(why);
            self.merge(r);
            return;
        }
        let text = b.label.as_deref().unwrap_or("!");
        let r = claim_press(ui.push_id(id, |ui| ui.button(text)).inner);
        let r = with_tip(r, b.tip.as_deref());
        if r.clicked()
            && let Some(edit) = &b.on
        {
            self.resp.edits.push((edit.clone(), Datum::Null));
        } else if r.clicked() {
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

    /// Render a grid of cells bound to rows-of-cells state. Cells are bools
    /// or numbers. Rows and columns come from the state shape. Any cell edit
    /// rebuilds and writes the whole value.
    fn matrix(&mut self, m: &Matrix, id: egui::Id, ctx: &mut dyn TreeState, ui: &mut egui::Ui) {
        let Some(bind) = &m.bind else {
            let r = unbound_chip(ui, "matrix");
            self.merge(r);
            return;
        };
        let path = self.resolve(bind);
        let Ok(Some(val)) = ctx.value_at(&path) else {
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
                                        let r = claim_press(ui.add_sized(size, button));
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
    /// selects how the producing node accumulates its state. It is
    /// irrelevant to drawing, so the walker ignores it.
    fn plot(
        &mut self,
        p: &gantz_ui::Plot,
        id: egui::Id,
        ctx: &mut dyn TreeState,
        ui: &mut egui::Ui,
    ) {
        let Some(bind) = &p.bind else {
            let r = unbound_chip(ui, "plot");
            self.merge(r);
            return;
        };
        let path = self.resolve(bind);
        let mut channels = match ctx.value_at(&path) {
            Ok(Some(val)) => plot::split_channels(&val),
            _ => Vec::new(),
        };
        if let Some(capacity) = p.capacity {
            for channel in &mut channels {
                let excess = channel.len().saturating_sub(capacity as usize);
                channel.drain(..excess);
            }
        }
        let params = plot::PlotParams {
            style: p.style.unwrap_or(gantz_ui::PlotStyle::Bars),
            color: p.color.map(|Rgba(c)| c),
            frame: plot::PlotFrame {
                grid: p.grid,
                axes: p.axes,
                interactive: p.interactive,
            },
            y_min: p.y_min,
            y_max: p.y_max,
        };
        // Absent dimensions fill the available space, as in the detached
        // view and the debug pane. Fragments with a fixed size set both.
        let size = egui::vec2(
            p.w.unwrap_or_else(|| ui.available_width()),
            p.h.unwrap_or_else(|| ui.available_height()),
        );
        let r = plot::plot_body(&params, &channels, id, size, ui);
        self.merge(r);
    }

    /// Render an envelope editor over the node that owns the tree. Each step
    /// of a drag is a live edit, and the drag's end or a menu edit is an
    /// edit. See [`envelope::edit`].
    fn envelope(&mut self, e: &EnvelopeElem, id: egui::Id, ui: &mut egui::Ui) {
        let Some(edit) = &e.on else {
            let r = err_label(ui, "an envelope needs an `on` edit");
            self.merge(r);
            return;
        };
        if !self.owned {
            let why = "an edit goes only to the node of its own view";
            let r = weak_chip(ui, "envelope").on_hover_text(why);
            self.merge(r);
            return;
        }
        let Some(env) = &e.value else {
            let r = err_label(ui, "an envelope needs a `value`");
            self.merge(r);
            return;
        };
        let duration = env.total_time();
        let view = envelope::View {
            grid: e.grid,
            axes: e.axes,
            x_range: e.x_range.unwrap_or(match duration > 0.0 {
                true => [0.0, duration],
                false => [0.0, 1.0],
            }),
            y_range: e.y_range.unwrap_or([0.0, 1.0]),
        };
        let size = egui::vec2(
            e.w.unwrap_or_else(|| ui.available_width()),
            e.h.unwrap_or_else(|| ui.available_height()),
        );
        let edited = envelope::edit(ui, id, env, size, view);
        let datum = |env: &gantz_ui::envelope::Envelope| gantz_ca::to_datum(env).ok();
        if let Some(value) = edited.live.as_ref().and_then(datum) {
            self.resp.live_edits.push((edit.clone(), value));
        }
        if let Some(value) = edited.commit.as_ref().and_then(datum) {
            self.resp.edits.push((edit.clone(), value));
        }
        self.merge(edited.response);
    }

    /// Render the figure that bound state holds. State that is not a figure
    /// draws as an empty panel.
    fn canvas(
        &mut self,
        c: &gantz_ui::Canvas,
        id: egui::Id,
        ctx: &mut dyn TreeState,
        ui: &mut egui::Ui,
    ) {
        let Some(bind) = &c.bind else {
            let r = unbound_chip(ui, "canvas");
            self.merge(r);
            return;
        };
        let state = ctx.value_at(&self.resolve(bind)).ok().flatten();
        let state = match &c.at {
            Some(at) => state.and_then(|s| state_at(&s, &at.0)),
            None => state,
        };
        let figs = match state {
            Some(val) => gantz_ui::decode_figures(&gantz_ui::codec::steel::lower(&val)),
            _ => gantz_ui::Figures::default(),
        };
        let frame = plot::PlotFrame {
            grid: c.grid,
            axes: c.axes,
            interactive: c.interactive,
        };
        // Absent dimensions fill the available space, as a plot does.
        let size = egui::vec2(
            c.w.unwrap_or_else(|| ui.available_width()),
            c.h.unwrap_or_else(|| ui.available_height()),
        );
        let r = canvas::canvas_body(frame, &figs, id, size, ui);
        self.merge(r);
    }

    /// Render an embedded reference GUI. Resolve the accumulated ref chain
    /// to the leaf graph's body marker. Read and decode the marker's stored
    /// tree. Render it inside a group frame under an implicit scope.
    fn ref_gui_elem(
        &mut self,
        rg: &gantz_ui::RefGui,
        id: egui::Id,
        ctx: &mut dyn TreeState,
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
                // Resolved before the implicit scope push below. The marker
                // lives inside the instance at `rg.id`.
                let path = self.resolve(&BindPath(vec![rg.id, marker_id]));
                match ctx.value_at(&path) {
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
                // The ref id becomes an implicit scope, so the embedded
                // tree's binds resolve inside the referenced instance. The
                // id salt is distinct from the scope arm's so a sibling
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
    fn emit_set(&mut self, ctx: &mut dyn TreeState, path: &[node::Id], val: SteelVal, push: bool) {
        if let Err(e) = ctx.set_value_at(path, val) {
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

impl TreeState for NodeCtx<'_> {
    fn value_at(&self, path: &[node::Id]) -> Result<Option<SteelVal>, SteelErr> {
        self.extract_value_at(path)
    }

    fn set_value_at(&mut self, path: &[node::Id], val: SteelVal) -> Result<(), SteelErr> {
        self.update_value_at(path, val)
    }
}

impl TreeState for Detached {
    fn value_at(&self, _path: &[node::Id]) -> Result<Option<SteelVal>, SteelErr> {
        Ok(None)
    }

    fn set_value_at(&mut self, path: &[node::Id], _val: SteelVal) -> Result<(), SteelErr> {
        let msg = format!("no node state to write at {path:?}");
        Err(SteelErr::new(steel::rerrs::ErrorKind::Generic, msg))
    }
}

/// A child's identity. It is the structural position beneath `parent`, or
/// the `key` attr when present. Key spaces are disjoint from position
/// spaces, so keyed and unkeyed siblings can never collide.
fn child_id(parent: egui::Id, position: usize, key: Option<&Key>) -> egui::Id {
    match key {
        Some(Key::Str(s)) => parent.with(("k", s.as_str())),
        Some(Key::Int(i)) => parent.with(("k", i)),
        None => parent.with(position),
    }
}

/// The full state-tree path of a binding. It is the tree's instance prefix,
/// then the accumulated scope prefixes, then the bind path itself.
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

/// A matrix value's cells as rows. `None` when the bound state is not
/// rows-of-cells shaped. Lists and vectors are accepted interchangeably.
fn matrix_rows(val: &SteelVal) -> Option<Vec<Vec<SteelVal>>> {
    let (_, rows) = seq_elems(val)?;
    rows.iter()
        .map(|row| seq_elems(row).map(|(_, cells)| cells))
        .collect()
}

/// Rebuild a whole matrix value with one cell replaced. The list-vs-vector
/// shape is kept at both levels. `None` when `val` is not a matrix or the
/// cell is out of bounds.
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

/// Whether a value is a steel list or vector. The two are interchangeable.
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

/// A drag value for the number `val`, with the dialer's bounds, precision,
/// step and prefix. A value that is not a number shows an error label.
pub(crate) fn drag_value(ui: &mut egui::Ui, d: &Dialer, val: &mut SteelVal) -> egui::Response {
    match val {
        SteelVal::NumV(f) => {
            let mut dv = egui::DragValue::new(f);
            if d.min.is_some() || d.max.is_some() {
                let (lo, hi) = (
                    d.min.unwrap_or(f64::NEG_INFINITY),
                    d.max.unwrap_or(f64::INFINITY),
                );
                dv = dv.range(lo..=hi);
            }
            match (fixed_decimals(d), d.precision) {
                (Some(n), _) => dv = dv.fixed_decimals(n),
                (None, Some(p)) => dv = dv.max_decimals(p as usize),
                (None, None) => (),
            }
            if let Some(digits) = fixed_digits(d) {
                dv =
                    dv.custom_formatter(move |v, decimals| zero_padded(v, digits, *decimals.end()));
            }
            if let Some(s) = drag_step(d) {
                dv = dv.speed(s);
            }
            if let Some(p) = &d.prefix {
                dv = dv.prefix(p);
            }
            if let Some(s) = &d.suffix {
                dv = dv.suffix(s);
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
            if let Some(digits) = fixed_digits(d) {
                dv = dv.custom_formatter(move |v, _| zero_padded(v, digits, 0));
            }
            if let Some(s) = drag_step(d) {
                dv = dv.speed(s);
            }
            if let Some(p) = &d.prefix {
                dv = dv.prefix(p);
            }
            if let Some(s) = &d.suffix {
                dv = dv.suffix(s);
            }
            ui.add(dv)
        }
        _ => err_label(ui, "bound state is not a number"),
    }
}

/// The bounds of a dialer bounded on both sides, while they are finite and
/// apart.
fn bounds(d: &Dialer) -> Option<(f64, f64)> {
    let (lo, hi) = (d.min?, d.max?);
    let span = hi - lo;
    (span.is_finite() && span > 0.0).then_some((lo, hi))
}

/// The decimals a dialer bounded on both sides always shows, so that it keeps
/// one width as its value moves: its precision, or else enough for its bounds
/// and its drag step. `None` for a dialer that is not bounded on both sides.
pub(crate) fn fixed_decimals(d: &Dialer) -> Option<usize> {
    let (lo, hi) = bounds(d)?;
    if let Some(p) = d.precision {
        return Some(usize::from(p));
    }
    let step = drag_step(d).unwrap_or(1.0);
    let step_decimals = (-step.log10()).ceil().max(0.0) as usize;
    let auto = decimals(lo).max(decimals(hi)).max(step_decimals);
    Some(auto.min(MAX_DECIMALS))
}

/// The integer digits a dialer bounded on both sides always shows: those of
/// its wider bound. Leading zeros fill the rest, so that it keeps one width
/// as its value moves. `None` for a dialer that is not bounded on both sides.
pub(crate) fn fixed_digits(d: &Dialer) -> Option<usize> {
    let (lo, hi) = bounds(d)?;
    let digits = |x: f64| format!("{:.0}", x.abs().trunc()).len();
    Some(digits(lo).max(digits(hi)))
}

/// How far a drag moves the dialer per point: its step, or else a share of
/// a range bounded on both sides, so that a drag of [`RANGE_DRAG`] points
/// crosses it. `None` leaves egui's own speed.
pub(crate) fn drag_step(d: &Dialer) -> Option<f64> {
    d.step
        .or_else(|| bounds(d).map(|(lo, hi)| (hi - lo) / RANGE_DRAG))
}

/// The decimals `x` needs, in its shortest form.
fn decimals(x: f64) -> usize {
    let s = x.to_string();
    s.split_once('.').map_or(0, |(_, frac)| frac.len())
}

/// `v` with `decimals` decimals, and leading zeros up to `digits` integer
/// digits.
fn zero_padded(v: f64, digits: usize, decimals: usize) -> String {
    let sign = if v.is_sign_negative() && v != 0.0 {
        "-"
    } else {
        ""
    };
    let width = match decimals {
        0 => digits,
        n => digits + 1 + n,
    };
    format!("{sign}{:0width$.decimals$}", v.abs())
}

/// Whether the tree `e` fills the space it is given: whether it holds a plot
/// or a canvas without a fixed width or height. A host gives such a tree a size, such as
/// a resizable node body or a pane.
pub(crate) fn fills(e: &Element) -> bool {
    match e {
        Element::Plot(p) => p.w.is_none() || p.h.is_none(),
        Element::Canvas(c) => c.w.is_none() || c.h.is_none(),
        e => e.children().iter().any(fills),
    }
}

/// Where `val` sits between the dialer's bounds, from 0 to 1. `None` unless
/// both bounds are set, finite and apart, and the value is a number.
pub(crate) fn range_fraction(d: &Dialer, val: &SteelVal) -> Option<f32> {
    let (lo, hi) = bounds(d)?;
    let v = match val {
        SteelVal::NumV(f) => *f,
        SteelVal::IntV(i) => *i as f64,
        _ => return None,
    };
    Some(((v - lo) / (hi - lo)).clamp(0.0, 1.0) as f32)
}

/// Fill `slot` with the fill of a bounded dialer in `rect`, from its left
/// edge to the fraction `t` of its width. The fill takes the dialer's
/// rounded shape, cut square at `t`.
fn paint_range_fill(ui: &egui::Ui, slot: egui::layers::ShapeIdx, rect: egui::Rect, t: f32) {
    let width = rect.width() * t;
    // A fill thinner than a point would draw as a hairline.
    if width < 1.0 {
        return;
    }
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let fill = egui::Shape::rect_filled(rect, radius, ui.visuals().selection.bg_fill);
    let cut = rect.with_max_x(rect.left() + width);
    ui.painter().with_clip_rect(cut).set(slot, fill);
}

/// Make the control `r` take the drag too. Inside a node, a press on a
/// control that takes only clicks would go on to select and move the node.
/// The widget's own `changed` mark carries over, as `interact` drops it.
fn claim_press(r: egui::Response) -> egui::Response {
    let changed = r.changed();
    let mut claimed = r.interact(egui::Sense::drag());
    if changed {
        claimed.mark_changed();
    }
    claimed
}

/// What a control binds, from its attributes.
fn binding<'b>(
    bind: &'b Option<BindPath>,
    at: &'b Option<At>,
    field: &'b Option<Field>,
    on: &'b Option<Edit>,
    value: Option<SteelVal>,
) -> Binding<'b> {
    Binding {
        bind,
        at,
        field,
        on,
        value,
    }
}

/// The response, with `tip` shown while the pointer hovers it.
fn with_tip(r: egui::Response, tip: Option<&str>) -> egui::Response {
    match tip {
        Some(tip) => r.on_hover_text(tip),
        None => r,
    }
}

/// An inline chip for a control missing its `bind` attr.
fn unbound_chip(ui: &mut egui::Ui, tag: &str) -> egui::Response {
    weak_chip(ui, tag).on_hover_text("no `bind` attribute")
}

/// The value at `keys` within the nested hash maps of `state`. See [`At`].
fn state_at(state: &SteelVal, keys: &[String]) -> Option<SteelVal> {
    keys.iter().try_fold(state.clone(), |val, key| {
        let SteelVal::HashMapV(map) = &val else {
            return None;
        };
        map.get(&map_key(map, key)).cloned()
    })
}

/// `state` with the value at `keys` replaced by `val`. The other entries of
/// each map stay. A missing map on the way, or a value that is not a map,
/// becomes a map. See [`At`].
fn with_state_at(state: Option<&SteelVal>, keys: &[String], val: SteelVal) -> SteelVal {
    let Some((key, rest)) = keys.split_first() else {
        return val;
    };
    let map = match state {
        Some(SteelVal::HashMapV(map)) => (**map).clone(),
        _ => HashMap::new(),
    };
    let key = map_key(&map, key);
    let inner = with_state_at(map.get(&key), rest, val);
    SteelVal::HashMapV(Gc::new(map.update(key, inner)).into())
}

/// The key of `map` named `name`. A string key when `map` has one and no
/// symbol key of the name, else a symbol key.
fn map_key(map: &HashMap<SteelVal, SteelVal>, name: &str) -> SteelVal {
    let symbol = SteelVal::SymbolV(name.into());
    let string = SteelVal::StringV(name.into());
    match !map.contains_key(&symbol) && map.contains_key(&string) {
        true => string,
        false => symbol,
    }
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
    fn fixed_decimals_fit_the_bounds_and_the_drag_step() {
        let dialer = |min, max, precision| Dialer {
            min,
            max,
            precision,
            ..Default::default()
        };
        assert_eq!(fixed_decimals(&dialer(Some(0.0), None, None)), None, "open");
        assert_eq!(
            fixed_decimals(&dialer(Some(0.0), Some(1.0), Some(2))),
            Some(2),
            "set"
        );
        assert_eq!(
            fixed_decimals(&dialer(Some(0.0), Some(1.0), None)),
            Some(3),
            "step"
        );
        assert_eq!(
            fixed_decimals(&dialer(Some(0.5), Some(256.0), None)),
            Some(1),
            "bound"
        );
    }

    #[test]
    fn a_tree_fills_where_a_plot_has_no_fixed_size() {
        let plot = |w, h| {
            Element::Plot(gantz_ui::Plot {
                w,
                h,
                ..Default::default()
            })
        };
        assert!(fills(&plot(None, None)));
        assert!(fills(&plot(Some(100.0), None)));
        assert!(!fills(&plot(Some(100.0), Some(50.0))));
        let col = |children| {
            Element::Col(gantz_ui::Col {
                children,
                ..Default::default()
            })
        };
        assert!(fills(&col(vec![plot(None, None)])));
        assert!(!fills(&col(vec![plot(Some(1.0), Some(1.0))])));
        assert!(!fills(&Element::Toggle(Toggle::default())));
        assert!(fills(&Element::Canvas(gantz_ui::Canvas::default())));
    }

    #[test]
    fn bounded_values_pad_to_the_digits_of_the_wider_bound() {
        let dialer = |min, max| Dialer {
            min,
            max,
            ..Default::default()
        };
        assert_eq!(fixed_digits(&dialer(Some(0.0), Some(100.0))), Some(3));
        assert_eq!(fixed_digits(&dialer(Some(-20.0), Some(1.5))), Some(2));
        assert_eq!(fixed_digits(&dialer(Some(0.0), None)), None, "open");
        assert_eq!(zero_padded(5.0, 3, 0), "005");
        assert_eq!(zero_padded(5.25, 3, 2), "005.25");
        assert_eq!(zero_padded(-5.0, 2, 1), "-05.0");
        assert_eq!(zero_padded(-0.0, 1, 1), "0.0");
    }

    #[test]
    fn range_fraction_needs_finite_apart_bounds() {
        let bounded = |min, max| Dialer {
            min,
            max,
            ..Default::default()
        };
        let d = bounded(Some(0.0), Some(10.0));
        assert_eq!(range_fraction(&d, &SteelVal::NumV(2.5)), Some(0.25));
        assert_eq!(range_fraction(&d, &SteelVal::IntV(20)), Some(1.0));
        assert_eq!(range_fraction(&d, &SteelVal::NumV(-1.0)), Some(0.0));
        assert_eq!(range_fraction(&d, &SteelVal::BoolV(true)), None);
        let open = bounded(Some(0.0), None);
        assert_eq!(range_fraction(&open, &SteelVal::NumV(1.0)), None);
        let flat = bounded(Some(1.0), Some(1.0));
        assert_eq!(range_fraction(&flat, &SteelVal::NumV(1.0)), None);
    }

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
    fn child_id_is_keyed_else_positional_beneath_the_parent() {
        let parent = egui::Id::new("parent");
        let (p1, p2) = (egui::Id::new("p1"), egui::Id::new("p2"));
        let a = Key::Str("a".to_string());
        let b = Key::Str("b".to_string());
        let cases = [
            (
                "a string key survives reorder",
                child_id(parent, 0, Some(&a)),
                child_id(parent, 5, Some(&a)),
                true,
            ),
            (
                "an int key survives reorder",
                child_id(parent, 0, Some(&Key::Int(3))),
                child_id(parent, 9, Some(&Key::Int(3))),
                true,
            ),
            (
                "unkeyed children differ by position",
                child_id(parent, 0, None),
                child_id(parent, 1, None),
                false,
            ),
            (
                "unkeyed children at one position are equal",
                child_id(parent, 2, None),
                child_id(parent, 2, None),
                true,
            ),
            (
                "distinct string keys are distinct",
                child_id(parent, 0, Some(&a)),
                child_id(parent, 0, Some(&b)),
                false,
            ),
            (
                "distinct int keys are distinct",
                child_id(parent, 0, Some(&Key::Int(1))),
                child_id(parent, 0, Some(&Key::Int(2))),
                false,
            ),
            (
                "unkeyed ids derive from the parent",
                child_id(p1, 0, None),
                child_id(p2, 0, None),
                false,
            ),
            (
                "keyed ids derive from the parent",
                child_id(p1, 0, Some(&a)),
                child_id(p2, 0, Some(&a)),
                false,
            ),
        ];
        for (case, x, y, equal) in cases {
            assert_eq!(x == y, equal, "{case}");
        }
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
            let graph = gantz_ca::DataGraph::default();
            let mut ctx =
                crate::NodeCtx::new(&env, &graph, &[][..], &[], &[], &[], &mut vm, &mut writes);
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

    /// Each nested embed extends the chain handed to the resolver. The
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

    /// Field-bound and edit-bound controls render from the given fields and
    /// their values, and report no edit without input. Without fields they
    /// render inert.
    #[test]
    fn field_controls_read_the_given_fields() {
        let field = |name: &str| Some(gantz_ui::Field(name.to_string()));
        let tree = Element::Col(gantz_ui::Col {
            children: vec![
                Element::Dialer(Dialer {
                    field: field("amount"),
                    ..Dialer::default()
                }),
                Element::TextInput(TextInput {
                    field: field("src"),
                    ..TextInput::default()
                }),
                Element::Dialer(Dialer {
                    on: Some(Edit("start".into())),
                    value: Some(0.25),
                    ..Dialer::default()
                }),
                Element::Button(Button {
                    on: Some(Edit("add".into())),
                    ..Button::default()
                }),
            ],
            ..gantz_ui::Col::default()
        });
        let fields = Datum::Map(vec![
            ("amount".into(), Datum::F64(0.5)),
            ("src".into(), Datum::Str("bd".into())),
        ]);
        run_with_ctx(vec![], |ctx, ui| {
            let r = UiTree::new(egui::Id::new("t"))
                .fields(&fields)
                .show(&tree, ctx, ui);
            assert!(r.inner.is_some());
            assert!(r.field_edits.is_empty() && r.edits.is_empty());
            let r = UiTree::new(egui::Id::new("u")).show(&tree, ctx, ui);
            assert!(r.inner.is_some(), "inert chips render");
            assert!(r.field_edits.is_empty() && r.edits.is_empty());
        });
    }

    /// A tree that no node owns renders over no state. Its edits still
    /// arrive, and a bound control shows that its path holds nothing.
    #[test]
    fn detached_trees_render_and_send_edits() {
        let tree = Element::Col(gantz_ui::Col {
            children: vec![
                Element::Dialer(Dialer {
                    on: Some(Edit("gain".into())),
                    value: Some(0.5),
                    ..Dialer::default()
                }),
                Element::Dialer(Dialer {
                    bind: Some(BindPath(vec![0])),
                    ..Dialer::default()
                }),
            ],
            ..gantz_ui::Col::default()
        });
        egui::__run_test_ui(|ui| {
            let r = UiTree::new(egui::Id::new("t"))
                .owned()
                .show(&tree, &mut Detached, ui);
            assert!(r.inner.is_some());
            assert!(r.edits.is_empty() && r.payloads.is_empty());
        });
        assert!(Detached.value_at(&[0]).unwrap().is_none());
        assert!(Detached.set_value_at(&[0], SteelVal::IntV(1)).is_err());
    }

    /// A text input commits its edit once typing pauses for `TEXT_FLUSH`, or
    /// as it loses focus, and not at each keystroke.
    #[test]
    fn text_input_commits_once_typing_pauses_or_on_focus_loss() {
        let ctx = egui::Context::default();
        let tree = Element::TextInput(TextInput {
            on: Some(Edit("src".into())),
            value: Some(String::new()),
            ..TextInput::default()
        });
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        let frame = |time: f64, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                events,
                time: Some(time),
                screen_rect: Some(screen),
                ..Default::default()
            };
            let mut out = None;
            let _ = ctx.run_ui(input, |ui| {
                let r = UiTree::new(egui::Id::new("t"))
                    .owned()
                    .show(&tree, &mut Detached, ui);
                out = Some((r.inner.map(|r| r.rect), r.edits));
            });
            out.expect("a frame")
        };
        let click = |pos: egui::Pos2| {
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            vec![egui::Event::PointerMoved(pos), press(true), press(false)]
        };
        let text = |s: &str| vec![egui::Event::Text(s.into())];
        let edit = |s: &str| vec![(Edit("src".into()), Datum::Str(s.into()))];

        let (rect, _) = frame(0.0, vec![]);
        let rect = rect.expect("the input renders");
        assert!(frame(0.1, click(rect.center())).1.is_empty());
        assert!(frame(0.2, text("bd")).1.is_empty(), "no commit per key");
        assert!(frame(1.0, vec![]).1.is_empty(), "typing has not paused");
        assert_eq!(frame(2.3, vec![]).1, edit("bd"), "typing paused");
        assert!(frame(2.4, vec![]).1.is_empty(), "one commit");

        assert!(frame(3.0, text("x")).1.is_empty());
        let away = egui::pos2(390.0, 290.0);
        assert_eq!(frame(3.1, click(away)).1, edit("x"), "focus left");
    }

    /// A key path reads a value within nested maps, and a write replaces only
    /// that value. A string key stays a string key.
    #[test]
    fn state_keys_read_and_write_within_maps() {
        let sym = |s: &str| SteelVal::SymbolV(s.into());
        let string = |s: &str| SteelVal::StringV(s.into());
        let map = |entries: Vec<(SteelVal, SteelVal)>| {
            let map: HashMap<SteelVal, SteelVal> = entries.into_iter().collect();
            SteelVal::HashMapV(Gc::new(map).into())
        };
        let keys = |ks: &[&str]| ks.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        let freq = map(vec![
            (sym("value"), SteelVal::NumV(220.0)),
            (sym("pending"), SteelVal::ListV(Default::default())),
        ]);
        let state = map(vec![
            (sym("freq"), freq),
            (string("width"), SteelVal::NumV(0.5)),
        ]);
        let at = |state: &SteelVal, ks: &[&str]| state_at(state, &keys(ks));
        assert_eq!(at(&state, &["freq", "value"]), Some(SteelVal::NumV(220.0)));
        assert_eq!(at(&state, &["width"]), Some(SteelVal::NumV(0.5)));
        assert_eq!(at(&state, &["freq", "nope"]), None);
        assert_eq!(at(&state, &["width", "value"]), None);

        let freq_keys = keys(&["freq", "value"]);
        let written = with_state_at(Some(&state), &freq_keys, SteelVal::NumV(440.0));
        assert_eq!(
            at(&written, &["freq", "value"]),
            Some(SteelVal::NumV(440.0))
        );
        assert!(
            at(&written, &["freq", "pending"]).is_some(),
            "a sibling stays"
        );
        assert_eq!(at(&written, &["width"]), Some(SteelVal::NumV(0.5)));

        let width = with_state_at(Some(&state), &keys(&["width"]), SteelVal::NumV(1.0));
        let SteelVal::HashMapV(width) = &width else {
            panic!("a map");
        };
        assert!(width.contains_key(&string("width")));
        assert!(!width.contains_key(&sym("width")));

        let fresh = with_state_at(None, &keys(&["a", "b"]), SteelVal::BoolV(true));
        assert_eq!(at(&fresh, &["a", "b"]), Some(SteelVal::BoolV(true)));
    }
}
