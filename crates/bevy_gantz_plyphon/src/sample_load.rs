//! Load a WAV file into a `~sample` node.
//!
//! A `~sample`'s inspector emits [`LoadSample`]. [`on_load_sample`] opens a
//! file dialog and decodes the chosen file off the frame in a task.
//! [`poll_sample_load`] stores the decoded audio in the registry's asset
//! store, assigns it to the node and commits the head's working graph. The
//! result is a normal structural commit, so undo, export and collab sync
//! carry the asset like any other edit. The same code runs natively and on
//! the web.

use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head;
use bevy_gantz_egui::{ForHead, GraphCache, NodeCodecRes, refresh_cache};
use gantz_ca as ca;
use gantz_core::node::graph::NodeIx;
use gantz_nodetag::NodeTag;
use gantz_plyphon::{AudioAsset, LoadSample, Sample};

/// The in-flight load of one `~sample`. Only one runs at a time.
#[derive(Resource)]
pub(crate) struct SampleLoadTask {
    /// The open head that holds the node.
    head: Entity,
    /// The node's path within the head's graph.
    path: Vec<gantz_core::node::Id>,
    /// The file dialog and decode. `None` when the dialog was cancelled.
    task: bevy_tasks::Task<Option<Result<AudioAsset, String>>>,
}

/// Open a file dialog for a `~sample`'s [`LoadSample`] request, and decode
/// the chosen WAV file in a task. A request while another load runs is
/// ignored.
pub(crate) fn on_load_sample(
    trigger: On<ForHead<LoadSample>>,
    task: Option<Res<SampleLoadTask>>,
    mut cmds: Commands,
) {
    if task.is_some() {
        return;
    }
    let event = trigger.event();
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Load Sample")
        .add_filter("WAV", &["wav"]);
    let task = bevy_tasks::AsyncComputeTaskPool::get().spawn(async move {
        let handle = dialog.pick_file().await?;
        let bytes = handle.read().await;
        Some(AudioAsset::from_wav(&bytes).map_err(|e| e.to_string()))
    });
    cmds.insert_resource(SampleLoadTask {
        head: event.head,
        path: event.data.path.clone(),
        task,
    });
}

/// Finish a [`SampleLoadTask`] once its task is ready. Store the asset,
/// assign it to the node and commit the head's working graph.
pub(crate) fn poll_sample_load(
    mut load: ResMut<SampleLoadTask>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
    mut cmds: Commands,
) {
    let Some(result) = bevy_tasks::futures::check_ready(&mut load.task) else {
        return;
    };
    cmds.remove_resource::<SampleLoadTask>();
    let asset = match result {
        None => return,
        Some(Ok(asset)) => asset,
        Some(Err(e)) => {
            log::error!("bevy_gantz_plyphon: sample load failed: {e}");
            return;
        }
    };
    let Ok(mut data) = heads.get_mut(load.head) else {
        log::error!("bevy_gantz_plyphon: sample load: the head is no longer open");
        return;
    };
    if let Err(e) = assign_sample(&mut data.working_graph.0, &load.path, &asset) {
        log::error!("bevy_gantz_plyphon: sample load: {e}");
        return;
    }
    gantz_plyphon::add_audio_asset(&mut registry, &asset);
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        load.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Replace the `~sample` node at `path` in `graph` with one that plays
/// `asset`. The node must still be a `~sample` at the same index, since the
/// graph can change while the file dialog is open. Only a root-level path is
/// supported, which is what the inspector emits.
fn assign_sample(
    graph: &mut ca::DataGraph,
    path: &[gantz_core::node::Id],
    asset: &AudioAsset,
) -> Result<(), String> {
    let &[ix] = path else {
        return Err(format!("unsupported node path {path:?}"));
    };
    let Some(node) = graph.node_weight_mut(NodeIx::new(ix)) else {
        return Err(format!("no node at index {ix}"));
    };
    if node.tag != <Sample as NodeTag>::TAG {
        return Err(format!("the node at index {ix} is no longer a `~sample`"));
    }
    *node = gantz_core::data::erase_node_typed(&Sample::from_asset(asset))
        .map_err(|e| format!("failed to erase the `~sample`: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset() -> AudioAsset {
        AudioAsset::from_interleaved(vec![0.5, -0.5, 0.25, -0.25], 2, 44_100.0)
    }

    fn graph_with(node: ca::NodeData) -> ca::DataGraph {
        let mut g = ca::DataGraph::default();
        g.add_node(node);
        g
    }

    #[test]
    fn assign_sample_replaces_the_node() {
        let empty = gantz_core::data::erase_node_typed(&Sample::default()).unwrap();
        let mut g = graph_with(empty);
        assign_sample(&mut g, &[0], &asset()).unwrap();
        let expected = gantz_core::data::erase_node_typed(&Sample::from_asset(&asset())).unwrap();
        assert_eq!(g[NodeIx::new(0)], expected);
        assert_eq!(
            g[NodeIx::new(0)].blobs,
            vec![(gantz_plyphon::BUFFER_SECTION.to_string(), asset().addr())],
            "the node references its asset blob",
        );
    }

    #[test]
    fn assign_sample_rejects_another_node_or_path() {
        let other = gantz_core::data::erase_node_typed(&gantz_plyphon::Buffer::default()).unwrap();
        let mut g = graph_with(other.clone());
        assert!(assign_sample(&mut g, &[0], &asset()).is_err());
        assert_eq!(g[NodeIx::new(0)], other, "a non-sample node is untouched");
        assert!(assign_sample(&mut g, &[5], &asset()).is_err());
        assert!(assign_sample(&mut g, &[0, 1], &asset()).is_err());
    }
}
