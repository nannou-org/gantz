//! The nodes of the types that domains define as data. See [`gantz_nodedef`].
//!
//! One Rust type, [`DefNode`], serves every node definition. A node holds its
//! definition and its fields, and stores as the tag of its definition and its
//! fields. The definition decodes the node from its fields once, when the node
//! is made, and describes it. The node keeps the description: its shape, its
//! name, its view and the decoded node.

use crate::node::{DynNode, Kind, NodeUiInstance};
use crate::ui_tree::{UiTree, UiTreeResponse};
use crate::widget::node_inspector::table_row_h;
use crate::{
    ContextMenuResponse, DynResponse, Env, InspectorRowsResponse, InspectorUiResponse, NodeCtx,
    NodeUi, NodeUiResponse, NodeViewResponse, SocketKind,
};
use gantz_ca::{ContentAddr, Datum, NodeData, SectionId};
use gantz_core::datum;
use gantz_core::node::{self, Conns, EvalConf, ExprError, ExprResult, MetaCtx, RegCtx};
use gantz_nodedef::{DynNodeDef, NodeDef};
use std::any::Any;
use std::borrow::Cow;
use std::sync::Arc;
use steel::SteelVal;

/// A node of a type that a domain defines as data. See the module docs.
#[derive(Clone)]
pub struct DefNode {
    def: Arc<dyn DynNodeDef>,
    fields: Datum,
    desc: Arc<Desc>,
}

/// Import a file into the node at `path` in the emitting head. The host lets
/// the user pick a file that the named importer reads, stores the blob that
/// the importer gives and sends the edit to the node. See [`gantz_ui::File`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportIntoNode {
    /// The path of the node within the graph of the head.
    pub path: Vec<node::Id>,
    /// The name of the importer.
    pub importer: String,
    /// The edit that the node applies.
    pub edit: String,
}

/// What a node keeps of the description that its definition gives.
struct Desc {
    meta: Meta,
    view: gantz_nodedef::View,
    node: Arc<dyn Any + Send + Sync>,
}

/// The host form of [`gantz_nodedef::Meta`].
struct Meta {
    name: String,
    description: String,
    inputs: usize,
    outputs: usize,
    stateful: bool,
    push: bool,
    pull: bool,
    requires: Vec<String>,
    branches: Vec<EvalConf>,
    blobs: Vec<(SectionId, ContentAddr)>,
}

impl DefNode {
    /// A node of the definition `def` with the fields `fields`, a map, as the
    /// definition decodes it.
    pub fn new(def: Arc<dyn DynNodeDef>, fields: &Datum) -> Result<Self, String> {
        let desc = def.describe(fields)?;
        if !matches!(desc.fields, Datum::Map(_)) {
            return Err(format!("{}: the fields are not a map", def.tag()));
        }
        let gantz_nodedef::Description {
            fields,
            meta,
            view,
            node,
        } = desc;
        let meta = Meta::new(def.tag(), meta)?;
        let desc = Arc::new(Desc { meta, view, node });
        Ok(DefNode { def, fields, desc })
    }

    /// The tag of the definition of the node.
    pub fn tag(&self) -> &str {
        self.def.tag()
    }

    /// The fields of the node, as a map.
    pub fn fields(&self) -> &Datum {
        &self.fields
    }

    /// The decoded node, if its type is `T`.
    pub fn node<T: Any>(&self) -> Option<&T> {
        self.desc.node.downcast_ref()
    }

    /// The body that the view of the node gives, if any. Its bind paths start
    /// from the node.
    pub fn body(&self) -> Option<&gantz_ui::Element> {
        self.desc.view.body.as_ref()
    }

    /// The stored form of the node.
    pub fn erase(&self) -> NodeData {
        let mut data = NodeData::new(self.tag(), self.fields.clone());
        data.blobs = self.desc.meta.blobs.clone();
        data.canonicalize();
        data
    }

    /// Applies the edit `edit` with the value `value`, which a control of the
    /// view sent. Gives whether the node changed.
    pub fn edit(&mut self, edit: &str, value: &Datum) -> Result<bool, String> {
        match self.def.edit(&self.fields, edit, value)? {
            Some(fields) => self.replace(&fields),
            None => Ok(false),
        }
    }

    /// Sets the top-level field `field` to `value`. Gives whether the node
    /// changed.
    pub fn set_field(&mut self, field: &str, value: Datum) -> Result<bool, String> {
        let mut fields = self.fields.clone();
        let Datum::Map(entries) = &mut fields else {
            return Err("the fields are not a map".into());
        };
        match entries.iter_mut().find(|(k, _)| k == field) {
            Some((_, v)) if *v == value => return Ok(false),
            Some((_, v)) => *v = value,
            None => entries.push((field.to_string(), value)),
        }
        self.replace(&fields)
    }

    /// The state that the node would have after the edit `edit` with `value`,
    /// given its `state`, without a change of the node. The host writes it
    /// while a gesture such as a drag is in progress, and applies the edit at
    /// the end of the gesture.
    pub fn preview(&self, edit: &str, value: &Datum, state: &SteelVal) -> Option<SteelVal> {
        let fields = self.def.edit(&self.fields, edit, value).ok()??;
        self.state(&fields, Some(state))
    }

    /// Replaces the node with the node of the fields `fields`. Gives whether
    /// the fields changed.
    fn replace(&mut self, fields: &Datum) -> Result<bool, String> {
        let next = Self::new(self.def.clone(), fields)?;
        let changed = next.fields != self.fields;
        *self = next;
        Ok(changed)
    }

    /// The state of a node with the fields `fields`, given the state that it
    /// has, or `None` to keep it. A state with no data form is kept.
    fn state(&self, fields: &Datum, state: Option<&SteelVal>) -> Option<SteelVal> {
        let state = match state.map(datum::of_steel).transpose() {
            Ok(state) => state,
            Err(e) => {
                log::error!("{}: the state of the node is not data: {e}", self.tag());
                return None;
            }
        };
        let next = self.def.state(fields, state.as_ref())?;
        Some(datum::to_steel(&next))
    }
}

impl Meta {
    fn new(tag: &str, meta: gantz_nodedef::Meta) -> Result<Self, String> {
        let count = |n: usize, what: &str| match n <= Conns::MAX {
            true => Ok(n),
            false => Err(format!("{tag}: {n} {what}, more than {}", Conns::MAX)),
        };
        let name = match meta.name.is_empty() {
            true => tag.to_owned(),
            false => meta.name,
        };
        let branch = |set: Vec<bool>| {
            Conns::try_from_slice(&set)
                .map(EvalConf::Set)
                .map_err(|_| format!("{tag}: a branch of {} outputs", set.len()))
        };
        Ok(Meta {
            name,
            description: meta.description,
            inputs: count(meta.inputs, "inputs")?,
            outputs: count(meta.outputs, "outputs")?,
            stateful: meta.stateful,
            push: meta.push,
            pull: meta.pull,
            requires: meta.requires,
            branches: meta
                .branches
                .into_iter()
                .map(branch)
                .collect::<Result<_, _>>()?,
            blobs: meta.blobs,
        })
    }
}

impl gantz_core::Node for DefNode {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        self.desc.meta.inputs
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        self.desc.meta.outputs
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        self.desc.meta.stateful
    }

    fn branches(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        self.desc.meta.branches.clone()
    }

    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        match self.desc.meta.push {
            true => vec![EvalConf::All],
            false => vec![],
        }
    }

    fn pull_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        match self.desc.meta.pull {
            true => vec![EvalConf::All],
            false => vec![],
        }
    }

    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> ExprResult {
        let inputs: Vec<Option<&str>> = ctx.inputs().iter().map(|i| i.as_deref()).collect();
        let outputs: Vec<bool> = ctx.outputs().iter().collect();
        let args = gantz_nodedef::ExprArgs {
            inputs: &inputs,
            outputs: &outputs,
            path: ctx.path(),
            args: ctx.args(),
        };
        let src = self
            .def
            .expr(&self.fields, args)
            .map_err(|e| ExprError::custom(format!("{}: {e}", self.tag())))?;
        node::parse_expr(&src)
    }

    fn required_modules(&self, _ctx: MetaCtx) -> Vec<String> {
        self.desc.meta.requires.clone()
    }

    fn required_blobs(&self) -> Vec<(SectionId, ContentAddr)> {
        self.desc.meta.blobs.clone()
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        if !self.desc.meta.stateful {
            return;
        }
        let path = ctx.path().to_vec();
        let vm = ctx.vm();
        let prev = node::state::extract_value(vm, &path).ok().flatten();
        let Some(state) = self.state(&self.fields, prev.as_ref()) else {
            return;
        };
        if let Err(e) = node::state::update_value(vm, &path, state) {
            log::error!("{}: the state of the node: {e}", self.tag());
        }
    }
}

impl NodeUi for DefNode {
    fn name(&self, _: &Env<'_>) -> Cow<'_, str> {
        Cow::Borrowed(&self.desc.meta.name)
    }

    fn description(&self) -> Option<&str> {
        let d = &self.desc.meta.description;
        (!d.is_empty()).then_some(d.as_str())
    }

    fn ui(&mut self, mut ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let desc = self.desc.clone();
        let Some(body) = &desc.view.body else {
            let name = desc.meta.name.as_str();
            let framed =
                uictx.framed(|ui, _sockets| ui.add(egui::Label::new(name).selectable(false)));
            return NodeUiResponse::new(framed);
        };
        let read_only = ctx.is_read_only();
        let id = uictx.egui_id().with("def-view");
        let mut tree = UiTreeResponse::default();
        let fields = self.fields.clone();
        let mut content = |ui: &mut egui::Ui| {
            tree = show(id, body, &fields, desc.meta.outputs, &mut ctx, ui);
            tree.inner.clone().unwrap_or_else(|| ui.response())
        };
        let framed = match &desc.view.resize {
            None => uictx.framed(|ui, _sockets| content(ui)),
            Some(resize) => {
                let mut size = body_size(&fields, resize);
                let (framed, resized) =
                    crate::node::resizable_body(uictx, &mut size, resize.margin, |ui, _| {
                        content(ui)
                    });
                if resized && !read_only {
                    tree.field_edits.extend(size_edits(resize, size));
                }
                framed
            }
        };
        let (changed, payloads) = apply(self, &mut ctx, vec![tree]);
        let mut resp = NodeUiResponse::new(framed);
        resp.set_changed(changed);
        resp.payloads.extend(payloads);
        resp
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let desc = self.desc.clone();
        let row_h = table_row_h(body.ui_mut());
        let mut trees = vec![];
        for (ix, row) in desc.view.rows.iter().enumerate() {
            body.row(row_h * row.lines.max(1) as f32, |mut cols| {
                cols.col(|ui| {
                    ui.label(&row.label);
                });
                cols.col(|ui| {
                    let id = ui.id().with(("def-row", ix));
                    trees.push(show(id, &row.ui, &self.fields, desc.meta.outputs, ctx, ui));
                });
            });
        }
        let (changed, payloads) = apply(self, ctx, trees);
        let mut resp = InspectorRowsResponse::default();
        resp.changed = changed;
        resp.payloads = payloads;
        resp
    }

    fn inspector_ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> InspectorUiResponse {
        let desc = self.desc.clone();
        let Some(inspector) = &desc.view.inspector else {
            return InspectorUiResponse::default();
        };
        let id = ui.id().with("def-inspector");
        let tree = show(id, inspector, &self.fields, desc.meta.outputs, &mut ctx, ui);
        let inner = tree.inner.clone();
        let (changed, payloads) = apply(self, &mut ctx, vec![tree]);
        let mut resp = InspectorUiResponse::default();
        resp.inner = inner;
        resp.changed = changed;
        resp.payloads = payloads;
        resp
    }

    fn view_ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
        let desc = self.desc.clone();
        let Some(body) = &desc.view.body else {
            return crate::default_view_ui(&ctx, ui);
        };
        let id = ui.id().with("def-pane");
        let tree = show(id, body, &self.fields, desc.meta.outputs, &mut ctx, ui);
        let inner = tree.inner.clone();
        let (changed, payloads) = apply(self, &mut ctx, vec![tree]);
        let mut resp = NodeViewResponse::default();
        resp.inner = inner;
        resp.changed = changed;
        resp.payloads = payloads;
        resp
    }

    fn view_no_margin(&self) -> bool {
        self.desc.view.fill
    }

    fn context_menu(&mut self, ctx: &mut NodeCtx, ui: &mut egui::Ui) -> ContextMenuResponse {
        let desc = self.desc.clone();
        let mut resp = ContextMenuResponse::default();
        if ctx.is_read_only() {
            return resp;
        }
        for item in &desc.view.menu {
            let mut checked = item.checked.unwrap_or(false);
            let mut clicked = match item.checked {
                Some(_) => ui.checkbox(&mut checked, item.label.as_str()),
                None => ui.button(item.label.as_str()),
            };
            if !item.tip.is_empty() {
                clicked = clicked.on_hover_text(item.tip.as_str());
            }
            if !clicked.clicked() {
                continue;
            }
            let value = match item.checked {
                Some(_) => Datum::Bool(checked),
                None => Datum::Null,
            };
            match &item.import {
                None => match self.edit(&item.edit, &value) {
                    Ok(changed) => resp.changed |= changed,
                    Err(e) => {
                        log::warn!("`{}`: rejected the edit `{}`: {e}", self.tag(), item.edit)
                    }
                },
                // The import is asynchronous. The host edits and commits the
                // node when the file is read, so this is not a change yet.
                Some(importer) => resp.payloads.push(DynResponse::new(ImportIntoNode {
                    path: ctx.path().to_vec(),
                    importer: importer.clone(),
                    edit: item.edit.clone(),
                })),
            }
            ui.close();
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<crate::SocketDoc> {
        let docs = match kind {
            SocketKind::Input => &self.desc.view.inputs,
            SocketKind::Output => &self.desc.view.outputs,
        };
        docs.get(ix).map(socket_doc)
    }

    fn show_state(&self) -> bool {
        !self.desc.view.hide_state
    }
}

/// The [`Kind`] of the nodes of the node type `D`.
pub fn def_kind<D: NodeDef>() -> Kind {
    dyn_def_kind(gantz_nodedef::of::<D>())
}

/// The [`Kind`] of the nodes of the definition `def`.
pub fn dyn_def_kind(def: Arc<dyn DynNodeDef>) -> Kind {
    let tag = def.tag().to_owned();
    Kind::new(tag, move |node_data| {
        let node = DefNode::new(def.clone(), &node_data.data).map_err(|e| {
            gantz_core::data::ReifyNodeError {
                tag: node_data.tag.clone(),
                source: <gantz_ca::DatumError as serde::de::Error>::custom(e),
            }
        })?;
        let node: DynNode = Box::new(node);
        Ok(NodeUiInstance::new(node, erase))
    })
}

/// Erases a node that the [`Kind`] of [`dyn_def_kind`] reified.
///
/// Panics if `node` is not a [`DefNode`]. That [`Kind`] pairs this eraser only
/// with the nodes that it reifies.
fn erase(node: &DynNode) -> Result<NodeData, gantz_core::data::EraseNodeError> {
    let node: &dyn gantz_core::Node = &**node;
    let any: &dyn Any = node;
    let node = any.downcast_ref::<DefNode>().expect("a def node");
    Ok(node.erase())
}

fn socket_doc(doc: &gantz_nodedef::SocketDoc) -> crate::SocketDoc {
    let socket = crate::SocketDoc::ty(doc.ty.clone());
    match doc.description.is_empty() {
        true => socket,
        false => socket.with_description(doc.description.clone()),
    }
}

/// The body size that the fields hold. A missing or invalid field takes the
/// axis of the default of the resize.
fn body_size(fields: &Datum, resize: &gantz_nodedef::Resize) -> [u16; 2] {
    let axis = |field: &str, default: u16| {
        let n = match fields.get(field) {
            Some(Datum::I64(n)) => u16::try_from(*n).ok(),
            Some(Datum::U64(n)) => u16::try_from(*n).ok(),
            _ => None,
        };
        n.unwrap_or(default)
    };
    [
        axis(&resize.width, resize.default[0]),
        axis(&resize.height, resize.default[1]),
    ]
}

/// The field edits that store a settled body `size`.
fn size_edits(resize: &gantz_nodedef::Resize, [w, h]: [u16; 2]) -> [(gantz_ui::Field, Datum); 2] {
    [
        (gantz_ui::Field(resize.width.clone()), Datum::I64(w.into())),
        (gantz_ui::Field(resize.height.clone()), Datum::I64(h.into())),
    ]
}

/// Draws one of the trees of a node. Bind paths start from the node itself.
fn show(
    id: egui::Id,
    tree: &gantz_ui::Element,
    fields: &Datum,
    outputs: usize,
    ctx: &mut NodeCtx,
    ui: &mut egui::Ui,
) -> UiTreeResponse {
    let path: Vec<node::Id> = ctx.path().to_vec();
    let n_outputs = |_: &[node::Id]| Some(outputs);
    UiTree::new(id)
        .instance_prefix(&path)
        .n_outputs(&n_outputs)
        .fields(fields)
        .show(tree, ctx, ui)
}

/// Applies the edits of the trees that a view showed, in order: the live
/// edits of each tree, then its field edits, then its edits. Gives whether
/// the node changed, and the payloads of the trees. An edit that the
/// definition rejects leaves the node as it was.
fn apply(
    node: &mut DefNode,
    ctx: &mut NodeCtx,
    trees: Vec<UiTreeResponse>,
) -> (bool, Vec<DynResponse>) {
    let mut changed = false;
    let mut payloads = vec![];
    for tree in trees {
        for (edit, value) in tree.live_edits {
            preview(node, ctx, &edit.0, &value);
        }
        for (field, value) in tree.field_edits {
            match node.set_field(&field.0, value) {
                Ok(edited) => changed |= edited,
                Err(e) => log::warn!("`{}`: rejected an edit of `{}`: {e}", node.tag(), field.0),
            }
        }
        for (edit, value) in tree.edits {
            match node.edit(&edit.0, &value) {
                Ok(edited) => changed |= edited,
                Err(e) => log::warn!("`{}`: rejected the edit `{}`: {e}", node.tag(), edit.0),
            }
        }
        for (edit, importer) in tree.imports {
            payloads.push(DynResponse::new(ImportIntoNode {
                path: ctx.path().to_vec(),
                importer,
                edit: edit.0,
            }));
        }
        payloads.extend(tree.payloads);
    }
    (changed, payloads)
}

/// Writes the state that `node` would have after the live edit `edit`,
/// locally, so that code that reads the state follows a gesture in progress.
/// See [`DefNode::preview`].
fn preview(node: &DefNode, ctx: &mut NodeCtx, edit: &str, value: &Datum) {
    let Ok(Some(prev)) = ctx.extract_value() else {
        return;
    };
    if let Some(next) = node.preview(edit, value, &prev) {
        let _ = ctx.update_value_local(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_nodedef::{ExprArgs, Meta as DefMeta};
    use serde::{Deserialize, Serialize};

    /// A node that scales its input by `gain`, keeps `gain` as its state, and
    /// has `outputs` outputs.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Gain {
        gain: i64,
        #[serde(default = "one")]
        outputs: usize,
    }

    impl NodeDef for Gain {
        const TAG: &'static str = "Gain";

        fn meta(&self) -> DefMeta {
            DefMeta {
                inputs: 1,
                outputs: self.outputs,
                stateful: true,
                ..DefMeta::default()
            }
        }

        fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
            let x = ctx.inputs[0].ok_or("no input")?;
            Ok(format!("(* {x} {})", self.gain))
        }

        fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
            match (edit, value) {
                ("gain", Datum::I64(gain)) => Some(Gain {
                    gain: *gain,
                    ..*self
                }),
                _ => None,
            }
        }

        fn init_state(&self) -> Option<Datum> {
            Some(Datum::I64(self.gain))
        }

        fn sync_state(&self, _state: &Datum) -> Option<Datum> {
            Some(Datum::I64(self.gain))
        }
    }

    fn one() -> usize {
        1
    }

    fn node(gain: i64) -> DefNode {
        let fields = gantz_nodedef::encode(&Gain { gain, outputs: 1 }).unwrap();
        DefNode::new(gantz_nodedef::of::<Gain>(), &fields).unwrap()
    }

    /// A def node reifies from its stored form and erases back to the same
    /// form, and gives its decoded node.
    #[test]
    fn a_def_node_round_trips_its_stored_form() {
        let fields = gantz_nodedef::encode(&Gain {
            gain: 2,
            outputs: 1,
        })
        .unwrap();
        let mut data = NodeData::new("Gain", fields);
        data.canonicalize();
        let codec = crate::node::NodeCodec::new(Default::default(), [def_kind::<Gain>()]);
        let inst = codec.reify(&data).unwrap();
        assert_eq!(inst.erase().unwrap(), data);
        let any: &dyn Any = &*inst.node as &dyn gantz_core::Node;
        let node = any.downcast_ref::<DefNode>().unwrap();
        assert_eq!(node.tag(), "Gain");
        assert_eq!(node.node::<Gain>().map(|g| g.gain), Some(2));
    }

    /// A field edit and an edit through the definition replace the node. An
    /// edit that the definition cannot decode leaves the node as it was.
    #[test]
    fn edits_replace_the_node() {
        let mut node = node(1);
        assert!(node.set_field("gain", Datum::I64(4)).unwrap());
        assert!(!node.set_field("gain", Datum::I64(4)).unwrap());
        assert!(node.set_field("gain", Datum::Str("x".into())).is_err());
        assert_eq!(node.node::<Gain>().map(|g| g.gain), Some(4));
        assert!(node.edit("gain", &Datum::I64(7)).unwrap());
        assert!(!node.edit("other", &Datum::Null).unwrap());
        assert_eq!(node.node::<Gain>().map(|g| g.gain), Some(7));
    }

    /// The preview of an edit gives the state that the edited node would
    /// have, with no change of the node.
    #[test]
    fn a_preview_gives_the_state_after_an_edit() {
        let node = node(1);
        let state = node.preview("gain", &Datum::I64(9), &SteelVal::IntV(1));
        assert_eq!(state, Some(SteelVal::IntV(9)));
        assert_eq!(node.node::<Gain>().map(|g| g.gain), Some(1));
    }

    /// A node with more outputs than a node can have is refused.
    #[test]
    fn too_many_outputs_are_refused() {
        let fields = gantz_nodedef::encode(&Gain {
            gain: 1,
            outputs: Conns::MAX + 1,
        })
        .unwrap();
        assert!(DefNode::new(gantz_nodedef::of::<Gain>(), &fields).is_err());
    }
}
