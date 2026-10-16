//! Observers for the graph edits that the GUI emits for a head.

use crate::{
    BuiltinNodes, ForHead, GraphCache, GraphView, GuiState, HeadNodeInstances, NodeCodecRes, env,
    refresh_cache,
};
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use bevy_gantz::Registry;
use bevy_gantz::head;
use bevy_log as log;
use gantz_ca as ca;
use steel::steel_vm::engine::Engine;

/// The data of an observer that edits an open head. See [`HeadEdit::apply`].
#[derive(SystemParam)]
pub struct HeadEdit<'w, 's> {
    pub(crate) registry: ResMut<'w, Registry>,
    pub(crate) cache: ResMut<'w, GraphCache>,
    pub(crate) builtins: Res<'w, BuiltinNodes>,
    pub(crate) codec: Res<'w, NodeCodecRes>,
    pub(crate) gui_state: ResMut<'w, GuiState>,
    pub(crate) vms: NonSendMut<'w, head::HeadVms>,
    pub(crate) cmds: Commands<'w, 's>,
    #[allow(clippy::type_complexity)]
    heads: Query<
        'w,
        's,
        (
            &'static mut head::HeadRef,
            &'static mut head::WorkingGraph,
            &'static mut GraphView,
            &'static mut HeadNodeInstances,
        ),
        With<head::OpenHead>,
    >,
}

/// Mutable access to one open head for an edit. See [`HeadEdit::apply`].
pub(crate) struct Edit<'a, 'w, 's> {
    pub registry: &'a mut Registry,
    pub cache: &'a mut GraphCache,
    pub builtins: &'a BuiltinNodes,
    pub codec: &'a NodeCodecRes,
    pub cmds: &'a mut Commands<'w, 's>,
    pub entity: Entity,
    pub head: &'a mut ca::Head,
    pub graph: &'a mut ca::DataGraph,
    pub view: &'a mut gantz_edit::SceneView,
    pub instances: &'a mut gantz_egui::node::NodeInstances,
    pub state: &'a mut gantz_egui::widget::gantz::OpenHeadState,
    /// `None` until the head first compiles, and after a failed init.
    pub vm: Option<&'a mut Engine>,
}

impl HeadEdit<'_, '_> {
    /// Apply `edit` to the open head `entity`, then commit its working graph
    /// if it changed. See `head::WorkingGraph`. `what` names the edit in
    /// logs. Returns `None` when the head or its GUI state is missing.
    pub(crate) fn apply<R>(
        &mut self,
        entity: Entity,
        what: &str,
        edit: impl FnOnce(&mut Edit) -> R,
    ) -> Option<R> {
        let Ok((mut head_ref, mut graph, mut view, mut instances)) = self.heads.get_mut(entity)
        else {
            log::error!("{what}: head not found for entity {entity:?}");
            return None;
        };
        let Some(state) = self.gui_state.open_heads.get_mut(&**head_ref) else {
            log::error!("{what}: GUI state not found for head");
            return None;
        };
        let out = edit(&mut Edit {
            registry: &mut self.registry,
            cache: &mut self.cache,
            builtins: &self.builtins,
            codec: &self.codec,
            cmds: &mut self.cmds,
            entity,
            head: &mut head_ref.0,
            graph: &mut graph,
            view: &mut view,
            instances: &mut instances.0,
            state,
            vm: self.vms.get_mut(&entity),
        });
        bevy_gantz::commit_working_graph(
            &mut self.registry,
            &mut self.cmds,
            entity,
            &mut head_ref.0,
            &graph.0,
        );
        refresh_cache(&self.registry, &mut self.cache, &self.codec.0);
        Some(out)
    }
}

impl Edit<'_, '_, '_> {
    /// The head's branch name, which edits check for reference cycles.
    pub fn editing(&self) -> Option<String> {
        match &*self.head {
            ca::Head::Branch(name) => Some(name.to_string()),
            ca::Head::Commit(_) => None,
        }
    }

    /// Re-register the whole working graph with the head's VM, so new nodes
    /// get their state. Registration is idempotent for existing nodes. An
    /// edit that merged graphs into the registry refreshes the cache first.
    pub fn register_graph(&mut self, what: &str) {
        refresh_cache(self.registry, self.cache, &self.codec.0);
        let Some(vm) = self.vm.as_deref_mut() else {
            return;
        };
        let node_reg = env(self.registry, self.cache, self.builtins, self.codec);
        let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
        match self.codec.0.reify_graph(self.graph) {
            Ok(g) => gantz_core::graph::register(&get_node, &g, &[], vm),
            Err(e) => log::error!("{what}: cannot re-register the graph: {e}"),
        }
    }
}

/// Handle create node payloads.
pub fn on_create_node(trigger: On<ForHead<gantz_egui::CreateNode>>, mut edits: HeadEdit) {
    let event = trigger.event();
    edits.apply(event.head, "CreateNode", |e| {
        let editing = e.editing();
        let Some(vm) = e.vm.as_deref_mut() else {
            log::error!("CreateNode: VM not found for entity {:?}", e.entity);
            return;
        };
        let node_reg = env(e.registry, e.cache, e.builtins, e.codec);
        let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
        gantz_egui::ops::create_node(
            node_reg.registry,
            editing.as_deref(),
            &e.codec.0,
            &get_node,
            |node_type| node_reg.create_node(node_type),
            e.graph,
            e.view,
            e.state,
            vm,
            event.data.clone(),
        );
    });
}

/// Handle replace node payloads.
pub fn on_replace_node(trigger: On<ForHead<gantz_egui::ReplaceNode>>, mut edits: HeadEdit) {
    let event = trigger.event();
    edits.apply(event.head, "ReplaceNode", |e| {
        let editing = e.editing();
        let Some(vm) = e.vm.as_deref_mut() else {
            log::error!("ReplaceNode: VM not found for entity {:?}", e.entity);
            return;
        };
        let node_reg = env(e.registry, e.cache, e.builtins, e.codec);
        let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
        gantz_egui::ops::replace_node(
            node_reg.registry,
            editing.as_deref(),
            &e.codec.0,
            &get_node,
            |node_type| node_reg.create_node(node_type),
            e.graph,
            e.state,
            vm,
            event.data.clone(),
        );
    });
}

/// Handle drop unknown data payloads. The rewrite commits like any edit, so
/// undo restores the dropped data.
pub fn on_drop_unknown_data(
    trigger: On<ForHead<gantz_egui::DropUnknownData>>,
    mut edits: HeadEdit,
) {
    let event = trigger.event();
    edits.apply(event.head, "DropUnknownData", |e| {
        gantz_egui::ops::drop_unknown_data(&e.codec.0, e.graph, event.data);
    });
}

/// Handle branch node payloads.
///
/// Creates a new commit with the same graph content, a new timestamp and the
/// original as parent. Inserts the new name and replaces the NamedRef node in
/// the working graph.
pub fn on_branch_node(trigger: On<ForHead<gantz_egui::BranchNode>>, mut edits: HeadEdit) {
    let event = trigger.event();
    edits.apply(event.head, "BranchNode", |e| {
        gantz_egui::ops::branch_node(
            e.registry,
            bevy_gantz::reg::timestamp(),
            e.graph,
            event.data.new_name.clone(),
            event.data.ca,
            &event.data.path,
        );
    });
}

/// Handle create nested graph payloads.
///
/// Commits a fresh empty graph named `<parent>:<n>`, where `<parent>` is the
/// head's branch name. Inserts a synced `NamedRef` to it in the head's
/// working graph. Requires the head to be named.
pub fn on_create_nested_graph(
    trigger: On<ForHead<gantz_egui::CreateNestedGraph>>,
    mut edits: HeadEdit,
) {
    let event = trigger.event();
    edits.apply(event.head, "CreateNestedGraph", |e| {
        let ca::Head::Branch(parent) = e.head else {
            log::warn!("CreateNestedGraph: name the graph before adding a nested graph");
            return;
        };
        gantz_egui::ops::create_nested_graph(
            e.registry,
            bevy_gantz::reg::timestamp(),
            e.graph,
            e.view,
            e.state,
            event.data.pos,
            parent,
        );
        // The fresh nested graph must be reified before its NamedRef resolves.
        refresh_cache(e.registry, e.cache, &e.codec.0);
    });
}

/// Handle inspect edge payloads.
pub fn on_inspect_edge(trigger: On<ForHead<gantz_egui::InspectEdge>>, mut edits: HeadEdit) {
    let event = trigger.event();
    edits.apply(event.head, "InspectEdge", |e| {
        let Some(vm) = e.vm.as_deref_mut() else {
            log::error!("InspectEdge: VM not found for entity {:?}", e.entity);
            return;
        };
        let node_reg = env(e.registry, e.cache, e.builtins, e.codec);
        let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
        gantz_egui::ops::inspect_edge(
            &e.codec.0,
            &get_node,
            || node_reg.create_node("inspect"),
            e.graph,
            e.view,
            vm,
            event.data.clone(),
        );
    });
}

/// Handle copy selection payloads.
///
/// Serializes the selected nodes and their registry dependencies to a
/// `.gantz` document and writes the result to the system clipboard via
/// [`bevy_egui::EguiClipboard`].
pub fn on_copy_nodes(
    trigger: On<ForHead<gantz_egui::CopyNodes>>,
    registry: Res<Registry>,
    codec: Res<NodeCodecRes>,
    mut clipboard: ResMut<bevy_egui::EguiClipboard>,
    mut heads: Query<(&mut head::WorkingGraph, &GraphView), With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok((wg, gv)) = heads.get_mut(event.head) else {
        log::error!("CopySelection: head not found for entity {:?}", event.head);
        return;
    };

    let text = gantz_egui::ops::copy_nodes(&registry, &wg, gv, &event.data.0, &codec.0);
    if let Some(text) = text {
        clipboard.set_text(&text);
    }
}

/// Handle paste selection payloads.
///
/// Resolves the clipboard text via [`bevy_egui::EguiClipboard`] when the
/// payload does not carry it. Parses it into a [`gantz_egui::export::Copied`],
/// merges registry dependencies, adds the subgraph, maps positions, and
/// selects the pasted nodes.
pub fn on_paste(
    trigger: On<ForHead<gantz_egui::Paste>>,
    mut clipboard: ResMut<bevy_egui::EguiClipboard>,
    mut edits: HeadEdit,
) {
    let event = trigger.event();
    let Some(text) = event.data.text.clone().or_else(|| clipboard.get_text()) else {
        return;
    };
    edits.apply(event.head, "Paste", |e| {
        let editing = e.editing();
        let pasted = gantz_egui::ops::paste(
            e.registry,
            editing.as_deref(),
            e.graph,
            e.view,
            e.state,
            &text,
            &event.data.pos,
            &e.codec.0,
        );
        if pasted {
            e.register_graph("Paste");
        }
    });
}

/// Handle cut payloads. Copy the selection to the clipboard, then remove it.
pub fn on_cut_nodes(
    trigger: On<ForHead<gantz_egui::CutNodes>>,
    mut clipboard: ResMut<bevy_egui::EguiClipboard>,
    mut edits: HeadEdit,
) {
    let event = trigger.event();
    edits.apply(event.head, "CutNodes", |e| {
        let Some(vm) = e.vm.as_deref_mut() else {
            log::error!("CutNodes: VM not found for head");
            return;
        };
        let text = gantz_egui::ops::cut_nodes(
            e.registry,
            e.graph,
            vm,
            e.view,
            &mut e.state.scene.interaction.selection,
            e.instances,
            &event.data.0,
            &e.codec.0,
        );
        if let Some(text) = text {
            clipboard.set_text(&text);
        }
    });
}

/// Handle nest payloads. Cut the selected nodes into a new nested graph node.
pub fn on_nest_nodes(trigger: On<ForHead<gantz_egui::NestNodes>>, mut edits: HeadEdit) {
    let event = trigger.event();
    edits.apply(event.head, "NestNodes", |e| {
        let ca::Head::Branch(parent) = e.head else {
            log::warn!("NestNodes: name the graph before nesting nodes");
            return;
        };
        let Some(vm) = e.vm.as_deref_mut() else {
            log::error!("NestNodes: VM not found for head");
            return;
        };
        gantz_egui::ops::nest_nodes(
            e.registry,
            bevy_gantz::reg::timestamp(),
            e.graph,
            vm,
            e.view,
            e.state,
            e.instances,
            &event.data.0,
            parent,
        );
        // The fresh nested graph must be reified before its NamedRef resolves.
        refresh_cache(e.registry, e.cache, &e.codec.0);
    });
}

/// Handle duplicate payloads. Copy the selection, then paste it at an offset.
pub fn on_duplicate_nodes(trigger: On<ForHead<gantz_egui::DuplicateNodes>>, mut edits: HeadEdit) {
    let event = trigger.event();
    edits.apply(event.head, "DuplicateNodes", |e| {
        let editing = e.editing();
        let duplicated = gantz_egui::ops::duplicate_nodes(
            e.registry,
            editing.as_deref(),
            e.graph,
            e.view,
            e.state,
            &event.data.0,
            &e.codec.0,
        );
        if duplicated {
            e.register_graph("DuplicateNodes");
        }
    });
}
