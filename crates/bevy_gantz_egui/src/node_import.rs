//! Imports a file into a [`DefNode`]. See [`gantz_nodedef::import`].
//!
//! A `(file)` control or a menu item in the view of a node emits
//! [`ImportIntoNode`]. [`on_import_into_node`] opens a file dialog for the
//! file types of the named importer, then reads and imports the chosen file
//! off the frame in a task. [`poll_node_import`] stores the blob in the
//! registry, sends the edit to the node and commits the working graph of the
//! head. The result is a normal commit, so undo, export and collab sync carry
//! the blob like any other edit. The same code runs natively and on the web.

use crate::{ForHead, GraphCache, NodeCodecRes, refresh_cache};
use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head::{OpenHead, OpenHeadData};
use bevy_log as log;
use gantz_ca::{BlobLiveness, Datum, SectionId};
use gantz_core::node::graph::NodeIx;
use gantz_egui::node::{DefNode, ImportIntoNode, NodeCodec};
use gantz_nodedef::import::Importer;

/// The importers of the domains of the app.
#[derive(Default, Resource)]
pub struct Importers(pub Vec<Importer>);

/// The import in flight. Only one runs at a time.
#[derive(Resource)]
pub struct NodeImportTask {
    /// The open head that holds the node.
    head: Entity,
    /// The path of the node within the graph of the head.
    path: Vec<gantz_core::node::Id>,
    /// The edit that the node applies.
    edit: String,
    /// The section of the blobs of the importer.
    section: SectionId,
    /// The file dialog and the import. `None` when the dialog was cancelled.
    task: bevy_tasks::Task<Option<Result<(Vec<u8>, Datum), String>>>,
}

/// Opens a file dialog for an [`ImportIntoNode`] request, and imports the
/// chosen file in a task. A request while another import runs is ignored.
pub fn on_import_into_node(
    trigger: On<ForHead<ImportIntoNode>>,
    importers: Res<Importers>,
    task: Option<Res<NodeImportTask>>,
    mut cmds: Commands,
) {
    if task.is_some() {
        return;
    }
    let event = trigger.event();
    let request = &event.data;
    let importer = importers.0.iter().find(|i| i.name == request.importer);
    let Some(importer) = importer.cloned() else {
        log::error!("import: no importer named `{}`", request.importer);
        return;
    };
    let extensions: Vec<&str> = importer.extensions.iter().map(String::as_str).collect();
    let dialog = rfd::AsyncFileDialog::new()
        .set_title(format!("Import {}", importer.description))
        .add_filter(&importer.description, &extensions);
    let section = importer.section.clone();
    let task = bevy_tasks::AsyncComputeTaskPool::get().spawn(async move {
        let handle = dialog.pick_file().await?;
        let bytes = handle.read().await;
        Some((importer.import)(&bytes))
    });
    cmds.insert_resource(NodeImportTask {
        head: event.head,
        path: request.path.clone(),
        edit: request.edit.clone(),
        section,
        task,
    });
}

/// Finishes a [`NodeImportTask`] once its task is ready. Stores the blob,
/// sends the edit to the node and commits the working graph of the head.
pub fn poll_node_import(
    mut import: ResMut<NodeImportTask>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut heads: Query<OpenHeadData, With<OpenHead>>,
    mut cmds: Commands,
) {
    let Some(result) = bevy_tasks::futures::check_ready(&mut import.task) else {
        return;
    };
    cmds.remove_resource::<NodeImportTask>();
    let (blob, data) = match result {
        None => return,
        Some(Ok(imported)) => imported,
        Some(Err(e)) => {
            log::error!("import failed: {e}");
            return;
        }
    };
    let Ok(mut head) = heads.get_mut(import.head) else {
        log::error!("import: the head is no longer open");
        return;
    };
    let liveness = BlobLiveness::ContentReferenced;
    let addr = registry.add_blob(import.section.clone(), liveness, blob);
    let value = gantz_nodedef::import::edit_value(addr, data);
    let graph = &mut head.working_graph.0;
    match edit_node(graph, &import.path, &import.edit, &value, &codec.0) {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            log::error!("import: {e}");
            return;
        }
    }
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        import.head,
        &mut head.head_ref.0,
        &head.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Sends `edit` with `value` to the node at `path` in `graph`, and stores the
/// edited node. Gives whether the node changed. Only a root path is
/// supported, which is what the view of a node emits. The graph can change
/// while the file dialog is open, so the node must still be a [`DefNode`]
/// that applies the edit.
fn edit_node(
    graph: &mut gantz_ca::DataGraph,
    path: &[gantz_core::node::Id],
    edit: &str,
    value: &Datum,
    codec: &NodeCodec,
) -> Result<bool, String> {
    let &[ix] = path else {
        return Err(format!("unsupported node path {path:?}"));
    };
    let Some(data) = graph.node_weight_mut(NodeIx::new(ix)) else {
        return Err(format!("no node at index {ix}"));
    };
    let inst = codec.reify(data).map_err(|e| e.to_string())?;
    let node: &dyn gantz_core::Node = &*inst.node;
    let node: &dyn std::any::Any = node;
    let Some(node) = node.downcast_ref::<DefNode>() else {
        return Err(format!("the node at index {ix} does not take imports"));
    };
    let mut node = node.clone();
    if !node.edit(edit, value)? {
        return Ok(false);
    }
    *data = node.erase();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_ca::{ContentAddr, DataGraph, NodeData};
    use gantz_nodedef::{ExprArgs, Meta, NodeDef};
    use serde::{Deserialize, Serialize};

    /// A node that keeps the address of the blob of its last import.
    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Clip {
        asset: Option<String>,
    }

    impl NodeDef for Clip {
        const TAG: &'static str = "Clip";

        fn meta(&self) -> Meta {
            Meta {
                outputs: 1,
                ..Meta::default()
            }
        }

        fn expr(&self, _ctx: ExprArgs<'_>) -> Result<String, String> {
            Ok("'()".into())
        }

        fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
            let (addr, _) = gantz_nodedef::import::imported(value).filter(|_| edit == "load")?;
            Some(Clip {
                asset: Some(addr.to_string()),
            })
        }
    }

    fn codec() -> NodeCodec {
        let kinds = [
            gantz_egui::node::def_kind::<Clip>(),
            gantz_egui::node::kind::<gantz_core::node::Expr>(),
        ];
        NodeCodec::new(Default::default(), kinds)
    }

    fn graph(node: NodeData) -> DataGraph {
        let mut g = DataGraph::default();
        g.add_node(node);
        g
    }

    fn clip(asset: Option<ContentAddr>) -> NodeData {
        let asset = asset.map(|addr| addr.to_string());
        gantz_nodedef::node_data(&Clip { asset }).unwrap()
    }

    /// The edit of an import stores the node as its definition edits it.
    #[test]
    fn an_import_edits_the_node() {
        let mut g = graph(clip(None));
        let addr = ContentAddr([7; 32]);
        let value = gantz_nodedef::import::edit_value(addr, Datum::Null);
        assert_eq!(edit_node(&mut g, &[0], "load", &value, &codec()), Ok(true));
        assert_eq!(g[NodeIx::new(0)], clip(Some(addr)));
        assert_eq!(
            edit_node(&mut g, &[0], "other", &value, &codec()),
            Ok(false)
        );
    }

    /// An import reaches only a root def node.
    #[test]
    fn an_import_needs_a_root_def_node() {
        let value = gantz_nodedef::import::edit_value(ContentAddr([7; 32]), Datum::Null);
        let expr = gantz_core::node::Expr::new("1").unwrap();
        let expr = gantz_core::data::erase_node_typed(&expr).unwrap();
        let mut g = graph(expr.clone());
        assert!(edit_node(&mut g, &[0], "load", &value, &codec()).is_err());
        assert_eq!(g[NodeIx::new(0)], expr, "the node is untouched");
        let mut g = graph(clip(None));
        assert!(edit_node(&mut g, &[5], "load", &value, &codec()).is_err());
        assert!(edit_node(&mut g, &[0, 1], "load", &value, &codec()).is_err());
    }
}
