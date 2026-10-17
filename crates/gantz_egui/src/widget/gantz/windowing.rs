//! Panes popped out into windows.

use super::state::{ViewToggles, load_ron, store_ron};
use super::tiles::{Pane, add_node_view_pane, set_pane_visible};

/// A stable identity for a pane, used to key its window and to dedupe the
/// windowed set. Singletons key on their variant. A node view keys on its
/// `(head, path)`, the same identity `add_node_view_pane` dedupes on. Public
/// so a native host can key a pop-out window's persisted geometry in
/// [`GantzState::windowed_geometry`] by the same identity.
///
/// [`GantzState::windowed_geometry`]: super::GantzState::windowed_geometry
pub fn pane_key(pane: &Pane) -> String {
    match pane {
        Pane::Ext(key) => format!("ext:{key}"),
        Pane::GraphConfig => "graph-config".to_string(),
        Pane::GraphScene => "graph-scene".to_string(),
        Pane::Graphs => "graphs".to_string(),
        Pane::GuiPreview => "gui-preview".to_string(),
        Pane::GuiTree => "gui-tree".to_string(),
        Pane::GuiPerf => "gui-perf".to_string(),
        Pane::History => "history".to_string(),
        Pane::Logs => "logs".to_string(),
        Pane::NodeInspector => "node-inspector".to_string(),
        Pane::Settings => "settings".to_string(),
        Pane::Steel => "steel".to_string(),
        Pane::VmPerf => "vm-perf".to_string(),
        Pane::NodeView(p) => {
            use std::fmt::Write;
            let mut s = format!("node-view:{}", p.head);
            for seg in &p.path {
                let _ = write!(s, ":{seg}");
            }
            s
        }
    }
}

/// egui-memory id under which the set of windowed panes persists, stored as a
/// RON `String` alongside the tile tree. See [`load_ron`].
pub(super) fn windowed_panes_id() -> egui::Id {
    egui::Id::new("gantz-windowed-panes-storage-v2")
}

/// egui-memory id for panes a host has requested be re-docked before the next
/// `Gantz::show`. See [`redock_windowed_pane`].
pub(super) fn pending_redock_id() -> egui::Id {
    egui::Id::new("gantz-pending-redock")
}

/// egui-memory id for node views a host has requested be closed before the
/// next `Gantz::show`. See [`close_windowed_pane`].
pub(super) fn pending_close_id() -> egui::Id {
    egui::Id::new("gantz-pending-close")
}

/// Load and clear a pending-pane list stored in egui memory.
pub(super) fn drain_pending(ctx: &egui::Context, id: egui::Id) -> Vec<Pane> {
    let pending: Vec<Pane> = load_ron(ctx, id).unwrap_or_default();
    if !pending.is_empty() {
        store_ron(ctx, id, &Vec::<Pane>::new());
    }
    pending
}

/// Append a pane to a pending-intent list in egui memory.
fn enqueue_pending(ctx: &egui::Context, id: egui::Id, pane: &Pane) {
    let mut pending: Vec<Pane> = load_ron(ctx, id).unwrap_or_default();
    pending.push(pane.clone());
    store_ron(ctx, id, &pending);
}

/// Request that a popped-out pane return to the tile tree, from outside a
/// [`Gantz::show`] call, for example a native host's OS-window close button.
///
/// The request is queued in egui memory and applied on the next `show`, so it
/// works from any egui context. A no-op if the pane is not windowed.
///
/// [`Gantz::show`]: super::Gantz::show
pub fn redock_windowed_pane(ctx: &egui::Context, pane: &Pane) {
    enqueue_pending(ctx, pending_redock_id(), pane);
}

/// Request that a popped-out node view be closed and destroyed rather than
/// re-docked. Queued and applied like [`redock_windowed_pane`]. Singletons
/// have no destructive close, so they are re-docked instead.
pub fn close_windowed_pane(ctx: &egui::Context, pane: &Pane) {
    enqueue_pending(ctx, pending_close_id(), pane);
}

/// Add `pane` to the windowed set unless one with the same identity is already
/// there.
fn push_windowed(windowed: &mut Vec<Pane>, pane: Pane) {
    let key = pane_key(&pane);
    if windowed.iter().all(|p| pane_key(p) != key) {
        windowed.push(pane);
    }
}

/// Pop a pane out of the tile tree into a window.
///
/// Node views are real tiles, so the tile is removed. Singletons stay in the
/// tree but hidden, since their window shows them instead. Either way the
/// pane joins the windowed set.
pub(super) fn detach_pane(
    view: &mut ViewToggles,
    windowed: &mut Vec<Pane>,
    tiles: &mut egui_tiles::Tiles<Pane>,
    tile_id: egui_tiles::TileId,
    pane: &Pane,
) {
    match pane {
        Pane::NodeView(_) => {
            tiles.remove(tile_id);
        }
        _ => set_pane_visible(view, pane, false),
    }
    push_windowed(windowed, pane.clone());
}

/// Return a windowed pane to the tile tree. A node view re-enters as a tray
/// tile. A singleton just becomes visible again, since its tile stayed in the
/// tree.
pub(super) fn redock_pane(view: &mut ViewToggles, tree: &mut egui_tiles::Tree<Pane>, pane: Pane) {
    match pane {
        Pane::NodeView(p) => add_node_view_pane(tree, p.head, p.path, p.ty_name),
        pane => set_pane_visible(view, &pane, true),
    }
}

/// Migrate windowed [`Pane::NodeView`] entries for `head` after a node removal,
/// mirroring [`migrate_node_view_paths`] for the windowed set. A view of a
/// removed node is dropped. A view of a swapped node has its path rewritten.
pub(super) fn migrate_windowed_node_views(
    windowed: &mut Vec<Pane>,
    head: &gantz_ca::Head,
    reindex: &crate::ops::Reindex,
) {
    if reindex.is_empty() {
        return;
    }
    windowed.retain_mut(|pane| {
        let Pane::NodeView(p) = pane else {
            return true;
        };
        if p.head != *head {
            return true;
        }
        let [ix] = p.path[..] else {
            return true;
        };
        match reindex.apply_to_index(ix) {
            Some(new_ix) => {
                p.path = vec![new_ix];
                true
            }
            None => false,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::super::tiles::{NodeViewPane, node_view};
    use super::*;
    use crate::node;

    /// Pane identity keys are stable per pane and independent of a node view's
    /// `ty_name`. Two views of the same node are the same identity.
    #[test]
    fn pane_key_identity() {
        assert_eq!(pane_key(&Pane::Logs), "logs");
        assert_ne!(pane_key(&Pane::Logs), pane_key(&Pane::Steel));
        assert_eq!(pane_key(&Pane::GuiPreview), "gui-preview");
        assert_eq!(pane_key(&Pane::GuiTree), "gui-tree");

        let plot = node_view("main", &[3]);
        let same_node_number = Pane::NodeView(NodeViewPane {
            head: gantz_ca::Head::Branch("main".parse().unwrap()),
            path: vec![3],
            ty_name: "number".to_string(),
        });
        assert_eq!(pane_key(&plot), pane_key(&same_node_number));
        assert_ne!(pane_key(&plot), pane_key(&node_view("main", &[4])));
        assert_ne!(pane_key(&plot), pane_key(&node_view("other", &[3])));
    }

    /// `push_windowed` dedupes by pane identity, so a repeated pop-out or a
    /// same-node view with a different `ty_name` does not add a second entry.
    #[test]
    fn push_windowed_dedupes() {
        let mut windowed = Vec::new();
        push_windowed(&mut windowed, Pane::Logs);
        push_windowed(&mut windowed, Pane::Logs);
        push_windowed(&mut windowed, node_view("main", &[3]));
        push_windowed(
            &mut windowed,
            Pane::NodeView(NodeViewPane {
                head: gantz_ca::Head::Branch("main".parse().unwrap()),
                path: vec![3],
                ty_name: "number".to_string(),
            }),
        );
        assert_eq!(windowed.len(), 2);
    }

    /// After a node removal, a windowed view of the removed node is dropped and a
    /// view of a swapped node has its path rewritten. Other heads and non-view
    /// panes are untouched.
    #[test]
    fn migrate_windowed_node_views_drops_and_rewrites() {
        let head = gantz_ca::Head::Branch("main".parse().unwrap());
        let mut windowed = vec![
            node_view("main", &[1]),  // removed node, dropped
            node_view("main", &[3]),  // swapped from 3 to 1
            node_view("main", &[2]),  // unaffected
            node_view("other", &[1]), // different head, untouched
            Pane::Logs,               // non-view, untouched
        ];
        // Node 1 is removed. The node that was at index 3 swaps down into
        // slot 1.
        let reindex = crate::ops::Reindex(vec![crate::ops::RemoveOp {
            removed: 1,
            moved_from: Some(3),
        }]);
        migrate_windowed_node_views(&mut windowed, &head, &reindex);

        let main_paths: Vec<Vec<node::Id>> = windowed
            .iter()
            .filter_map(|p| match p {
                Pane::NodeView(nv) if nv.head == head => Some(nv.path.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(main_paths, vec![vec![1], vec![2]]);
        assert!(windowed.iter().any(|p| matches!(p, Pane::Logs)));
        assert!(
            windowed
                .iter()
                .any(|p| matches!(p, Pane::NodeView(nv) if matches!(&nv.head, gantz_ca::Head::Branch(n) if n.to_string() == "other")))
        );
        assert_eq!(windowed.len(), 4);
    }
}
