use crate::node;
use std::collections::BTreeMap;

/// A rose-tree for graphs whose nodes may nest other graphs.
#[derive(Debug, Default)]
pub(crate) struct RoseTree<T> {
    pub(crate) elem: T,
    pub(crate) nested: BTreeMap<node::Id, RoseTree<T>>,
}

impl<T> RoseTree<T> {
    /// Get the tree (or nested tree) at the given nesting path.
    ///
    /// An empty list returns the top-level tree.
    pub(crate) fn tree(&self, path: &[node::Id]) -> Option<&Self> {
        let n_id = match path.first() {
            None => return Some(self),
            Some(&n_id) => n_id,
        };
        let tree = self.nested.get(&n_id)?;
        tree.tree(&path[1..])
    }

    /// Get the tree (or nested tree) at the given nesting path.
    ///
    /// An empty list returns the top-level tree.
    pub(crate) fn tree_mut(&mut self, path: &[node::Id]) -> &mut Self
    where
        T: Default,
    {
        let n_id = match path.first() {
            None => return self,
            Some(&n_id) => n_id,
        };
        let tree = self.nested.entry(n_id).or_default();
        tree.tree_mut(&path[1..])
    }
}
