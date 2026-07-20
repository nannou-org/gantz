//! A per-head cache of reified working-graph node instances.
//!
//! The working graph stores nodes as [`NodeData`]. Rendering needs the typed
//! form, so each pass reifies node weights through the app's
//! [`NodeCodec`] - a datum clone, a serde decode and a box allocation per
//! node. [`NodeInstances`] retains those instances between passes so the
//! steady-state cost per node is one structural equality check.
//!
//! Correctness rests on the [`NodeUi`](crate::NodeUi) contract: an instance
//! is a pure function of the `NodeData` it was reified from (all non-CA
//! state lives in the VM or egui memory), so reusing a cached instance is
//! indistinguishable from a fresh reify. Each entry therefore carries the
//! `NodeData` it was reified from as its validity witness: the entry is
//! valid exactly while the stored weight equals that witness. External
//! mutations of the graph (undo, paste, collab edits, checkout) need no
//! hooks - a rewritten weight simply misses and reifies fresh.

use crate::node::{NodeCodec, NodeUiInstance};
use gantz_ca::NodeData;
use gantz_core::data::ReifyNodeError;
use std::collections::HashMap;

/// A cached reified instance paired with the weight it was reified from.
pub struct InstanceEntry {
    /// The `NodeData` this instance was reified from - the validity witness.
    /// After an edit, set this to the newly erased data before
    /// [`put`](NodeInstances::put)ting the entry back.
    pub src: NodeData,
    /// The reified instance and its eraser.
    pub inst: NodeUiInstance,
}

/// A cache of reified node instances for one head's working graph, keyed by
/// node index.
///
/// An entry is valid exactly while the stored weight equals its
/// [`src`](InstanceEntry::src) witness. Index-keyed like the other per-node
/// working-graph state (VM state, layout, selection), it must be migrated
/// through the same [`Reindex`](crate::ops::Reindex) replay on node removal -
/// see [`apply_reindex`](Self::apply_reindex). A missed migration is safe
/// (the witness check catches it) but wastes a reify.
#[derive(Default)]
pub struct NodeInstances {
    entries: HashMap<usize, InstanceEntry>,
}

impl NodeInstances {
    /// Remove and return the entry for the node at index `n_ix`: the cached
    /// entry when its witness equals `data`, otherwise a freshly reified one
    /// (with `src` cloned from `data`). Any stale entry is dropped either
    /// way.
    ///
    /// `Err` is the codec's decode failure (e.g. an unknown tag) - the
    /// caller's placeholder path. No entry is retained on failure.
    pub fn take(
        &mut self,
        codec: &NodeCodec,
        n_ix: usize,
        data: &NodeData,
    ) -> Result<InstanceEntry, ReifyNodeError> {
        if let Some(entry) = self.entries.remove(&n_ix) {
            if entry.src == *data {
                return Ok(entry);
            }
        }
        let inst = codec.reify_ui(data)?;
        Ok(InstanceEntry {
            src: data.clone(),
            inst,
        })
    }

    /// Restore an entry after its pass, making it available to the next
    /// [`take`](Self::take).
    pub fn put(&mut self, n_ix: usize, entry: InstanceEntry) {
        self.entries.insert(n_ix, entry);
    }

    /// A read-only lookup that hits only on a valid cached entry: `None` on
    /// miss or stale witness. For probes that should not pay a reify - the
    /// caller decides whether to fall back to a transient one.
    pub fn peek(&self, n_ix: usize, data: &NodeData) -> Option<&NodeUiInstance> {
        let entry = self.entries.get(&n_ix)?;
        (entry.src == *data).then_some(&entry.inst)
    }

    /// Replay a [`remove_nodes`](crate::ops::remove_nodes) reindex onto the
    /// cache keys: removed nodes' entries are dropped and swapped nodes'
    /// entries follow them to their new index, mirroring
    /// [`Reindex::apply_to_index`](crate::ops::Reindex::apply_to_index).
    pub fn apply_reindex(&mut self, reindex: &crate::ops::Reindex) {
        for op in &reindex.0 {
            self.entries.remove(&op.removed);
            if let Some(from) = op.moved_from {
                if let Some(entry) = self.entries.remove(&from) {
                    self.entries.insert(op.removed, entry);
                }
            }
        }
    }

    /// Drop all entries. The next pass reifies every node fresh - use when
    /// the whole graph is replaced (e.g. head checkout) to bound memory.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Reindex, RemoveOp};

    fn codec() -> NodeCodec {
        crate::test_node::codec()
    }

    fn expr_data(src: &str) -> NodeData {
        gantz_core::data::erase_node_typed(&gantz_core::node::Expr::new(src).unwrap()).unwrap()
    }

    /// The cached instance's heap address - stable across `Box` moves, so it
    /// witnesses whether a take returned the same instance or a fresh one.
    fn inst_ptr(entry: &InstanceEntry) -> *const () {
        &*entry.inst.node as *const dyn crate::NodeUi as *const ()
    }

    // A take after a put with an equal witness returns the same instance.
    #[test]
    fn take_hits_on_equal_data() {
        let codec = codec();
        let data = expr_data("(+ $l $r)");
        let mut cache = NodeInstances::default();
        let entry = cache.take(&codec, 0, &data).unwrap();
        let ptr = inst_ptr(&entry);
        cache.put(0, entry);
        let entry = cache.take(&codec, 0, &data).unwrap();
        assert_eq!(inst_ptr(&entry), ptr);
        // The instance still erases back to the witness.
        assert_eq!(entry.inst.erase().unwrap(), data);
    }

    // A differing weight reifies fresh and drops the stale entry.
    #[test]
    fn take_misses_on_changed_data() {
        let codec = codec();
        let old = expr_data("(+ $l $r)");
        let new = expr_data("(* $l $r)");
        let mut cache = NodeInstances::default();
        let entry = cache.take(&codec, 0, &old).unwrap();
        cache.put(0, entry);
        // Simulates any external mutation (undo, paste, collab, checkout):
        // the stored weight changed out-of-band, no hooks involved. The
        // fresh reify is witnessed semantically (the dropped stale box's
        // address may be reused, so pointer inequality would be flaky).
        let entry = cache.take(&codec, 0, &new).unwrap();
        assert_eq!(entry.inst.erase().unwrap(), new);
        assert_eq!(entry.src, new);
        cache.put(0, entry);
        assert_eq!(cache.len(), 1);
    }

    // An unknown tag fails without retaining anything; the slot still works
    // for a later known tag.
    #[test]
    fn take_err_retains_nothing() {
        let codec = codec();
        let mut unknown = expr_data("(+ $l $r)");
        unknown.tag = "NotInTheManifest".to_string();
        let mut cache = NodeInstances::default();
        assert!(cache.take(&codec, 0, &unknown).is_err());
        assert!(cache.is_empty());
        let known = expr_data("(+ $l $r)");
        let entry = cache.take(&codec, 0, &known).unwrap();
        cache.put(0, entry);
        assert_eq!(cache.len(), 1);
    }

    // An unknown tag's failure also evicts a stale entry for that slot.
    #[test]
    fn take_err_drops_stale_entry() {
        let codec = codec();
        let known = expr_data("(+ $l $r)");
        let mut cache = NodeInstances::default();
        let entry = cache.take(&codec, 0, &known).unwrap();
        cache.put(0, entry);
        let mut unknown = expr_data("(+ $l $r)");
        unknown.tag = "NotInTheManifest".to_string();
        assert!(cache.take(&codec, 0, &unknown).is_err());
        assert!(cache.is_empty());
    }

    // The edit path: erase to new data, update the witness, put. The entry
    // then hits on the new data and misses on the old.
    #[test]
    fn edit_updates_witness() {
        let codec = codec();
        let old = expr_data("(+ $l $r)");
        let new = expr_data("(* $l $r)");
        let mut cache = NodeInstances::default();
        let mut entry = cache.take(&codec, 0, &old).unwrap();
        // Stand-in for an in-place UI edit of the instance.
        entry.inst = codec.reify_ui(&new).unwrap();
        entry.src = entry.inst.erase().unwrap();
        let ptr = inst_ptr(&entry);
        cache.put(0, entry);
        // Hits on the new data with the same instance.
        let entry = cache.take(&codec, 0, &new).unwrap();
        assert_eq!(inst_ptr(&entry), ptr);
        cache.put(0, entry);
        // Misses on the old data: the fresh reify erases back to `old`,
        // whereas the (now dropped) cached instance would have erased to
        // `new`. Pointer inequality would be flaky under allocator reuse.
        let entry = cache.take(&codec, 0, &old).unwrap();
        assert_eq!(entry.inst.erase().unwrap(), old);
    }

    // `peek` hits only on a valid witness and never mutates the cache.
    #[test]
    fn peek_checks_witness() {
        let codec = codec();
        let data = expr_data("(+ $l $r)");
        let other = expr_data("(* $l $r)");
        let mut cache = NodeInstances::default();
        assert!(cache.peek(0, &data).is_none());
        let entry = cache.take(&codec, 0, &data).unwrap();
        cache.put(0, entry);
        assert!(cache.peek(0, &data).is_some());
        assert!(cache.peek(0, &other).is_none());
        assert!(cache.peek(1, &data).is_none());
        assert_eq!(cache.len(), 1);
    }

    // Reindex replay: the removed slot's entry is dropped and the swapped
    // node's entry follows it down, consistent with `Reindex::apply_to_index`.
    #[test]
    fn apply_reindex_migrates_entries() {
        let codec = codec();
        let datas: Vec<_> = ["(+ $l $r)", "(* $l $r)", "(- $l $r)"]
            .iter()
            .map(|s| expr_data(s))
            .collect();
        let mut cache = NodeInstances::default();
        let mut ptrs = vec![];
        for (i, d) in datas.iter().enumerate() {
            let entry = cache.take(&codec, i, d).unwrap();
            ptrs.push(inst_ptr(&entry));
            cache.put(i, entry);
        }
        // Remove index 1 from a 3-node graph: node 2 swaps down into slot 1.
        let reindex = Reindex(vec![RemoveOp {
            removed: 1,
            moved_from: Some(2),
        }]);
        cache.apply_reindex(&reindex);
        assert_eq!(cache.len(), 2);
        assert_eq!(reindex.apply_to_index(2), Some(1));
        // Slot 1 now hits on node 2's data with node 2's instance.
        let entry = cache.take(&codec, 1, &datas[2]).unwrap();
        assert_eq!(inst_ptr(&entry), ptrs[2]);
        cache.put(1, entry);
        // Slot 0 is untouched, slot 2 is gone.
        assert!(cache.peek(0, &datas[0]).is_some());
        assert!(cache.peek(2, &datas[2]).is_none());
    }

    #[test]
    fn clear_empties() {
        let codec = codec();
        let data = expr_data("(+ $l $r)");
        let mut cache = NodeInstances::default();
        let entry = cache.take(&codec, 0, &data).unwrap();
        cache.put(0, entry);
        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }
}
