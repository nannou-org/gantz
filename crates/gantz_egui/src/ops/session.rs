//! Head history stepping: plain navigation undo/redo and the session
//! (revert-commit) counterparts, which mint forward revert commits so an
//! undo propagates to peers like any other edit.

use gantz_ca::CommitAddr;
use std::collections::HashMap;

/// Undo: push the head's current commit onto its redo stack and return the
/// parent commit to navigate to.
///
/// Returns `None` when the head has no parent commit to return to.
/// Navigation itself is frontend-specific and stays with the caller.
pub fn undo(
    registry: &gantz_ca::Registry,
    redo_stacks: &mut HashMap<gantz_ca::Head, Vec<CommitAddr>>,
    head: &gantz_ca::Head,
) -> Option<CommitAddr> {
    let commit_ca = registry.head_commit_ca(head)?;
    let parent = registry.commits().get(&commit_ca)?.parent?;
    redo_stacks.entry(head.clone()).or_default().push(commit_ca);
    Some(parent)
}

/// Redo: pop the most recently undone commit from the head's redo stack.
///
/// Navigation itself is frontend-specific and stays with the caller.
pub fn redo(
    redo_stacks: &mut HashMap<gantz_ca::Head, Vec<CommitAddr>>,
    head: &gantz_ca::Head,
) -> Option<CommitAddr> {
    redo_stacks.get_mut(head)?.pop()
}

/// Mint a forward *revert* commit: a new commit whose parent is `tip` and
/// whose graph is `target`'s, without moving any head or name.
///
/// This is the durable form of undo for shared sessions: navigating a head
/// backwards presents peers an ancestor tip (dropped as up-to-date by
/// design), whereas a revert commit propagates like any other edit. A
/// same-graph revert still mints - undoing a layout-only commit must move
/// the tip so node positions revert on peers too.
///
/// The graph already exists in the registry, so nothing is re-hashed or
/// cloned. Returns `None` when `tip` or `target` is missing from the
/// registry. Moving the head, views and the working-graph refresh stay with
/// the caller (see [`session_undo`] / [`session_redo`]).
pub(crate) fn revert_commit(
    registry: &mut gantz_ca::Registry,
    timestamp: gantz_ca::Timestamp,
    tip: CommitAddr,
    target: CommitAddr,
) -> Option<CommitAddr> {
    registry.commits().get(&tip)?;
    let target_graph = registry.commits().get(&target)?.graph;
    Some(
        registry.commit_graph(timestamp, Some(tip), target_graph, || {
            unreachable!("revert reuses an existing graph")
        }),
    )
}

/// The stepping state for revert-commit undo (see [`session_undo`]).
///
/// A revert commit's parent is the pre-revert tip, so plain parent-stepping
/// would oscillate: a second consecutive undo would target the first
/// revert's parent - the very tip the first undo left. The cursor records
/// where stepping stands in the *original* history; it counts only while
/// [`RevertCursor::minted`] is still the head's tip, so any other commit (an
/// edit, a remote merge, a navigation) invalidates it automatically.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RevertCursor {
    /// The revert commit last minted for this head.
    pub minted: CommitAddr,
    /// The historical commit whose graph that revert restored - the head's
    /// current position in the original history.
    pub target: CommitAddr,
}

/// Session undo: mint a forward revert commit (see `revert_commit`)
/// stepping one commit back through the head's original history, copy the
/// restored commit's stored view to the minted commit (substituting the live
/// camera), and record the stepping state.
///
/// The step base is the head's cursor position while the cursor is current
/// (`cursor.minted == tip`), else the tip itself; the revert target is that
/// base's parent. The base (the pre-undo history position) is pushed onto
/// the head's redo stack - [`session_redo`] mints a revert back to it.
///
/// Returns the minted commit for the caller to navigate the head to; `None`
/// at the history horizon (no parent - e.g. wire-truncated history) or when
/// the head is unresolvable.
pub fn session_undo(
    registry: &mut gantz_ca::Registry,
    redo_stacks: &mut HashMap<gantz_ca::Head, Vec<CommitAddr>>,
    undo_cursors: &mut HashMap<gantz_ca::Head, RevertCursor>,
    timestamp: gantz_ca::Timestamp,
    head: &gantz_ca::Head,
    live_camera: Option<crate::Camera>,
) -> Option<CommitAddr> {
    let tip = registry.head_commit_ca(head)?;
    let base = undo_cursors
        .get(head)
        .filter(|c| c.minted == tip)
        .map(|c| c.target)
        .unwrap_or(tip);
    let target = registry.commits().get(&base)?.parent?;
    let minted = revert_commit(registry, timestamp, tip, target)?;
    copy_view(registry, target, minted, live_camera);
    redo_stacks.entry(head.clone()).or_default().push(base);
    undo_cursors.insert(head.clone(), RevertCursor { minted, target });
    Some(minted)
}

/// Session redo: pop the most recently undone history position from the
/// head's redo stack and mint a forward revert commit restoring it (the
/// session counterpart of [`redo`]; see [`session_undo`]).
///
/// Returns the minted commit for the caller to navigate the head to.
pub fn session_redo(
    registry: &mut gantz_ca::Registry,
    redo_stacks: &mut HashMap<gantz_ca::Head, Vec<CommitAddr>>,
    undo_cursors: &mut HashMap<gantz_ca::Head, RevertCursor>,
    timestamp: gantz_ca::Timestamp,
    head: &gantz_ca::Head,
    live_camera: Option<crate::Camera>,
) -> Option<CommitAddr> {
    let tip = registry.head_commit_ca(head)?;
    let target = redo_stacks.get_mut(head)?.pop()?;
    let minted = revert_commit(registry, timestamp, tip, target)?;
    copy_view(registry, target, minted, live_camera);
    undo_cursors.insert(head.clone(), RevertCursor { minted, target });
    Some(minted)
}

/// Copy `src`'s stored view to `dst` (a freshly minted revert commit),
/// substituting the live camera so the viewport doesn't jump. Empty-layout
/// views are never stored (an adopting peer would auto-layout them).
fn copy_view(
    registry: &mut gantz_ca::Registry,
    src: CommitAddr,
    dst: CommitAddr,
    live_camera: Option<crate::Camera>,
) {
    let Some(mut view) = crate::section::view(registry, &src) else {
        return;
    };
    if view.layout.is_empty() {
        return;
    }
    if let Some(camera) = live_camera {
        view.camera = camera;
    }
    crate::section::set_view(registry, dst, &view);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::node_id;
    use crate::ops::test_util::*;

    // Session undo: the previous graph is committed *forward*.
    #[test]
    fn revert_commit_mints_previous_graph_forward() {
        let secs = |s| std::time::Duration::from_secs(s);
        let mut reg = gantz_ca::Registry::default();
        let g1 = test_graph(&[1]);
        let g1_ca = gantz_ca::graph_addr(&g1);
        let c1 = reg.commit_graph(secs(1), None, g1_ca, || g1);
        let g2 = test_graph(&[1, 2]);
        let g2_ca = gantz_ca::graph_addr(&g2);
        let c2 = reg.commit_graph(secs(2), Some(c1), g2_ca, || g2);
        // A layout-only commit: same graph, new commit.
        let c3 = reg.commit_graph(secs(3), Some(c2), g2_ca, || unreachable!("graph exists"));
        reg.set_head("alpha".parse().unwrap(), c3);

        let reverted = revert_commit(&mut reg, secs(4), c3, c1).unwrap();
        let commit = &reg.commits()[&reverted];
        // The revert is a new forward commit carrying the old graph; no head
        // or name moved.
        assert_eq!(commit.parent, Some(c3));
        assert_eq!(commit.graph, g1_ca);
        assert_eq!(reg.head(&"alpha".parse().unwrap()), Some(c3));
        // A same-graph revert still mints (a layout-only undo must move the
        // tip so positions revert on peers).
        let again = revert_commit(&mut reg, secs(5), c3, c2).unwrap();
        assert_eq!(reg.commits()[&again].graph, g2_ca);
        assert_eq!(reg.commits()[&again].parent, Some(c3));
    }

    // The cursor keeps undo/undo/redo/redo stepping through the original
    // history even though every step mints a fresh forward revert commit.
    #[test]
    fn session_undo_redo_stepping() {
        let secs = |s| std::time::Duration::from_secs(s);
        let mut reg = gantz_ca::Registry::default();
        // e1 -> l (layout-only) -> e2.
        let g1 = test_graph(&[1]);
        let g1_ca = gantz_ca::graph_addr(&g1);
        let e1 = reg.commit_graph(secs(1), None, g1_ca, || g1);
        let l = reg.commit_graph(secs(2), Some(e1), g1_ca, || unreachable!("graph exists"));
        let g2 = test_graph(&[1, 2]);
        let g2_ca = gantz_ca::graph_addr(&g2);
        let e2 = reg.commit_graph(secs(3), Some(l), g2_ca, || g2);
        reg.set_head("alpha".parse().unwrap(), e2);
        let head = gantz_ca::Head::Branch("alpha".parse().unwrap());

        // Stored views for the history commits; none for the mints yet.
        let view = |x: f32| {
            let mut v = crate::SceneView::default();
            v.layout.insert(node_id(0), egui::pos2(x, 0.0));
            v
        };
        crate::section::set_view(&mut reg, e1, &view(1.0));
        crate::section::set_view(&mut reg, l, &view(2.0));
        crate::section::set_view(&mut reg, e2, &view(3.0));
        let stored =
            |reg: &gantz_ca::Registry, ca: CommitAddr| crate::section::view(reg, &ca).unwrap();
        let mut redo = HashMap::new();
        let mut cursors = HashMap::new();
        let cam = crate::Camera {
            center: egui::pos2(9.0, 9.0),
            zoom: 2.0,
        };

        let navigate = |reg: &mut gantz_ca::Registry, minted| {
            reg.set_head("alpha".parse().unwrap(), minted);
        };

        // Undo 1: restores l's graph (== e1's content).
        let r1 = session_undo(
            &mut reg,
            &mut redo,
            &mut cursors,
            secs(10),
            &head,
            Some(cam),
        )
        .unwrap();
        navigate(&mut reg, r1);
        assert_eq!(reg.commits()[&r1].graph, g1_ca);
        assert_eq!(reg.commits()[&r1].parent, Some(e2));
        // The restored commit's view was copied, live camera substituted.
        assert_eq!(stored(&reg, r1).layout, stored(&reg, l).layout);
        assert_eq!(stored(&reg, r1).camera, cam);

        // Undo 2: steps to e1 through the cursor (NOT back to r1's parent).
        let r2 = session_undo(
            &mut reg,
            &mut redo,
            &mut cursors,
            secs(11),
            &head,
            Some(cam),
        )
        .unwrap();
        navigate(&mut reg, r2);
        assert_eq!(reg.commits()[&r2].graph, g1_ca);
        assert_eq!(reg.commits()[&r2].parent, Some(r1));
        assert_eq!(stored(&reg, r2).layout, stored(&reg, e1).layout);

        // Undo 3: at the history horizon - no-op.
        assert_eq!(
            session_undo(
                &mut reg,
                &mut redo,
                &mut cursors,
                secs(12),
                &head,
                Some(cam)
            ),
            None,
        );

        // Redo 1: back to the l position.
        let r3 = session_redo(
            &mut reg,
            &mut redo,
            &mut cursors,
            secs(13),
            &head,
            Some(cam),
        )
        .unwrap();
        navigate(&mut reg, r3);
        assert_eq!(reg.commits()[&r3].graph, g1_ca);
        assert_eq!(reg.commits()[&r3].parent, Some(r2));
        assert_eq!(stored(&reg, r3).layout, stored(&reg, l).layout);

        // Undo after redo: steps back to e1 (the cursor tracks the original
        // history position, not the revert commits' parents).
        let r4 = session_undo(
            &mut reg,
            &mut redo,
            &mut cursors,
            secs(14),
            &head,
            Some(cam),
        )
        .unwrap();
        navigate(&mut reg, r4);
        assert_eq!(stored(&reg, r4).layout, stored(&reg, e1).layout);
        // Redo back to l, then redo to e2: the full round trip.
        let r5 = session_redo(
            &mut reg,
            &mut redo,
            &mut cursors,
            secs(15),
            &head,
            Some(cam),
        )
        .unwrap();
        navigate(&mut reg, r5);
        assert_eq!(stored(&reg, r5).layout, stored(&reg, l).layout);
        let r6 = session_redo(
            &mut reg,
            &mut redo,
            &mut cursors,
            secs(16),
            &head,
            Some(cam),
        )
        .unwrap();
        navigate(&mut reg, r6);
        assert_eq!(reg.commits()[&r6].graph, g2_ca);
        assert_eq!(stored(&reg, r6).layout, stored(&reg, e2).layout);
        assert!(redo.get(&head).is_none_or(|s| s.is_empty()));
    }

    // A real edit on top of a revert invalidates the cursor: the next undo
    // steps from the new tip, restoring the pre-edit (reverted) state.
    #[test]
    fn session_undo_cursor_invalidated_by_edit() {
        let secs = |s| std::time::Duration::from_secs(s);
        let mut reg = gantz_ca::Registry::default();
        let g1 = test_graph(&[1]);
        let e1 = reg.commit_graph(
            secs(1),
            None,
            gantz_ca::graph_addr(&test_graph(&[1])),
            || g1,
        );
        let g2 = test_graph(&[1, 2]);
        let g2_ca = gantz_ca::graph_addr(&g2);
        let e2 = reg.commit_graph(secs(2), Some(e1), g2_ca, || g2);
        reg.set_head("alpha".parse().unwrap(), e2);
        let head = gantz_ca::Head::Branch("alpha".parse().unwrap());

        let mut redo = HashMap::new();
        let mut cursors = HashMap::new();

        // Undo to e1's state.
        let r1 = session_undo(&mut reg, &mut redo, &mut cursors, secs(10), &head, None).unwrap();
        reg.set_head("alpha".parse().unwrap(), r1);

        // A real edit on top of the revert; the committed machinery clears
        // the redo stack (mirrored here), and the cursor is stale by tip.
        let g3 = test_graph(&[1, 3]);
        let g3_ca = gantz_ca::graph_addr(&g3);
        let e3 = reg.commit_graph(secs(11), Some(r1), g3_ca, || g3);
        reg.set_head("alpha".parse().unwrap(), e3);
        redo.remove(&head);

        // Undo now steps from e3, restoring the pre-edit revert state.
        let r2 = session_undo(&mut reg, &mut redo, &mut cursors, secs(12), &head, None).unwrap();
        assert_eq!(reg.commits()[&r2].parent, Some(e3));
        assert_eq!(reg.commits()[&r2].graph, reg.commits()[&r1].graph);
    }
}
