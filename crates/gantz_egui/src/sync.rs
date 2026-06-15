//! Keeping `NamedRef` references current across the registry.
//!
//! When a named graph is edited it commits to a new address; every graph that
//! references it by name must then follow. [`resync`] brings all sync-enabled
//! [`NamedRef`](crate::node::NamedRef)s up to their name's current commit,
//! recommitting any graph whose references changed. This is the headless
//! counterpart of the inspector's render-time auto-sync, and the mechanism by
//! which editing a nested graph propagates up to its parents.

use crate::node::{NESTED_SEP, NamedRef};
use gantz_ca::{CaHash, CommitAddr, Registry};
use gantz_core::node::graph::Graph;
use std::collections::HashMap;
use std::time::Duration;

/// Access the [`NamedRef`] within a frontend's node type, if any.
///
/// Implemented by frontends (typically a downcast) so the otherwise node-type-
/// agnostic [`resync`] / rename machinery can find and update references. This
/// is the references-only analogue of the removed `ToGraphMut`.
pub trait AsNamedRefMut {
    /// A mutable reference to the inner [`NamedRef`], if this node is one.
    fn as_named_ref_mut(&mut self) -> Option<&mut NamedRef>;
}

/// A named graph whose commit moved during [`resync`] or a rename cascade.
#[derive(Clone, Debug)]
pub struct Moved {
    /// The name whose commit moved.
    pub name: String,
    /// The commit the name pointed at before.
    pub old_commit: CommitAddr,
    /// The commit the name points at now.
    pub new_commit: CommitAddr,
}

/// The number of `:`-separated segments in a name (a nested graph's depth).
fn depth(name: &str) -> usize {
    name.matches(NESTED_SEP).count()
}

/// Give a freshly-forked graph independent nested children.
///
/// Forking `old` to `new` copies `old`'s graph (done by the caller), but that
/// copy still references `old`'s nested children (`old:*`). This copies the
/// whole `old:*` subtree to `new:*` and rewrites the references so editing the
/// fork's nested graphs no longer affects the original. Returns the named
/// graphs whose commits were created or moved (the `new` root plus each
/// `new:*` child), so callers can refresh the open fork and migrate views.
///
/// Children are copied deepest-first so a parent's references resolve to its
/// already-copied children.
pub fn fork_nested<N>(
    registry: &mut Registry<Graph<N>>,
    timestamp: Duration,
    old: &str,
    new: &str,
) -> Vec<Moved>
where
    N: Clone + CaHash + AsNamedRefMut,
{
    let old_prefix = format!("{old}{NESTED_SEP}");
    let mut descendants: Vec<String> = registry
        .names()
        .keys()
        .filter(|n| n.starts_with(&old_prefix))
        .cloned()
        .collect();
    descendants.sort_by(|a, b| depth(b).cmp(&depth(a)).then_with(|| a.cmp(b)));

    // old descendant name -> (new name, new commit).
    let mut remap: HashMap<String, (String, CommitAddr)> = HashMap::new();
    let mut moves = Vec::new();

    // Rewrites refs to already-copied descendants; returns whether it changed.
    let rewrite = |graph: &mut Graph<N>, remap: &HashMap<String, (String, CommitAddr)>| {
        let mut changed = false;
        for weight in graph.node_weights_mut() {
            if let Some(named_ref) = weight.as_named_ref_mut() {
                if let Some((new_name, new_commit)) = remap.get(named_ref.name()) {
                    named_ref.rename(new_name.clone(), (*new_commit).into());
                    changed = true;
                }
            }
        }
        changed
    };

    for d in &descendants {
        let d_new = format!("{new}{}", &d[old.len()..]);
        let Some(&commit) = registry.names().get(d) else {
            continue;
        };
        let Some(graph) = registry.commit_graph_ref(&commit) else {
            continue;
        };
        let mut g = graph.clone();
        rewrite(&mut g, &remap);
        let graph_ca = gantz_ca::graph_addr(&g);
        let new_commit = registry.commit_graph_to_name(timestamp, graph_ca, || g, &d_new);
        remap.insert(d.clone(), (d_new.clone(), new_commit));
        moves.push(Moved {
            name: d_new,
            old_commit: commit,
            new_commit,
        });
    }

    // Rewrite the already-created `new` root's references to its own children.
    if let Some(&root_commit) = registry.names().get(new) {
        if let Some(graph) = registry.commit_graph_ref(&root_commit) {
            let mut g = graph.clone();
            if rewrite(&mut g, &remap) {
                let graph_ca = gantz_ca::graph_addr(&g);
                let new_commit = registry.commit_graph_to_name(timestamp, graph_ca, || g, new);
                moves.push(Moved {
                    name: new.to_string(),
                    old_commit: root_commit,
                    new_commit,
                });
            }
        }
    }

    moves
}

/// Bring every sync-enabled [`NamedRef`] in the registry up to its name's
/// current commit, recommitting any named graph whose references changed.
///
/// Returns the named graphs whose commits moved, so callers can refresh open
/// heads and migrate their views.
///
/// Graphs are processed deepest-name-first so a parent observes its children's
/// new commits within a single pass; a bounded fixpoint loop covers any
/// non-nesting reference shape. The loop cannot run forever even for a
/// (degenerate) mutually-referencing registry - it simply stops once no graph
/// changes.
pub fn resync<N>(registry: &mut Registry<Graph<N>>, timestamp: Duration) -> Vec<Moved>
where
    N: Clone + CaHash + AsNamedRefMut,
{
    // Deepest names first: a child is updated before the parent that refs it.
    let mut order: Vec<String> = registry.names().keys().cloned().collect();
    order.sort_by(|a, b| depth(b).cmp(&depth(a)).then_with(|| a.cmp(b)));

    // A name -> current commit snapshot, kept in step with commits we make so a
    // referrer resolves its children to their freshly-committed addresses.
    let mut current: HashMap<String, CommitAddr> = registry
        .names()
        .iter()
        .map(|(n, ca)| (n.clone(), *ca))
        .collect();

    let mut moves = Vec::new();
    let max_passes = order.len() + 1;
    for _ in 0..max_passes {
        let mut changed_any = false;
        for name in &order {
            let Some(&commit_ca) = current.get(name) else {
                continue;
            };
            let Some(graph) = registry.commit_graph_ref(&commit_ca) else {
                continue;
            };
            let mut new_graph = graph.clone();

            let resolve = |m: &str| current.get(m).copied().map(gantz_ca::ContentAddr::from);
            let mut changed = false;
            for weight in new_graph.node_weights_mut() {
                if let Some(named_ref) = weight.as_named_ref_mut() {
                    changed |= named_ref.resync(&resolve);
                }
            }

            if changed {
                let graph_ca = gantz_ca::graph_addr(&new_graph);
                let new_commit =
                    registry.commit_graph_to_name(timestamp, graph_ca, || new_graph, name);
                current.insert(name.clone(), new_commit);
                moves.push(Moved {
                    name: name.clone(),
                    old_commit: commit_ca,
                    new_commit,
                });
                changed_any = true;
            }
        }
        if !changed_any {
            break;
        }
    }
    moves
}

/// Promote a nested graph that was renamed to a (root) name: repoint its
/// parent's references from the old nested name to `new_name`, then drop the
/// now-orphaned nested name and its descendants.
///
/// `old_nested` is the renamed graph's former `parent:child` name; `new_name`
/// is its new root name (a fresh copy of its graph already committed under it).
/// The parent may hold *several* references to the nested graph - each an
/// independent instance with its own state - and they are all repointed.
/// Returns the parent's move (if it changed) so an open parent head can be
/// refreshed. A no-op (empty) when `old_nested` is not a nested name.
pub fn promote_nested<N>(
    registry: &mut Registry<Graph<N>>,
    timestamp: Duration,
    old_nested: &str,
    new_name: &str,
) -> Vec<Moved>
where
    N: Clone + CaHash + AsNamedRefMut,
{
    // The parent referencing the nested graph: the name with the last leaf
    // stripped (`A:1` -> `A`, `A:1:2` -> `A:1`).
    let Some((parent, _)) = old_nested.rsplit_once(NESTED_SEP) else {
        return Vec::new();
    };
    let parent = parent.to_string();
    let (Some(&new_commit), Some(&parent_commit)) = (
        registry.names().get(new_name),
        registry.names().get(&parent),
    ) else {
        return Vec::new();
    };

    // Repoint every parent reference (each a distinct instance) to the new name.
    let mut moves = Vec::new();
    if let Some(graph) = registry.commit_graph_ref(&parent_commit) {
        let mut g = graph.clone();
        let mut changed = false;
        for weight in g.node_weights_mut() {
            if let Some(named_ref) = weight.as_named_ref_mut() {
                if named_ref.name() == old_nested {
                    named_ref.rename(new_name.to_string(), new_commit.into());
                    changed = true;
                }
            }
        }
        if changed {
            let graph_ca = gantz_ca::graph_addr(&g);
            let moved = registry.commit_graph_to_name(timestamp, graph_ca, || g, &parent);
            moves.push(Moved {
                name: parent,
                old_commit: parent_commit,
                new_commit: moved,
            });
        }
    }

    // Drop the orphaned nested name and its descendants (their content survives
    // as the new root graph copy).
    let child_prefix = format!("{old_nested}{NESTED_SEP}");
    let orphans: Vec<String> = registry
        .names()
        .keys()
        .filter(|n| n.as_str() == old_nested || n.starts_with(&child_prefix))
        .cloned()
        .collect();
    for orphan in orphans {
        registry.remove_name(&orphan);
    }

    moves
}
