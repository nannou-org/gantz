use crate::node;
use std::collections::BTreeMap;

/// A rose-tree for graphs whose nodes may nest other graphs.
#[derive(Debug, Default)]
pub(crate) struct RoseTree<T> {
    pub(crate) elem: T,
    nested: BTreeMap<node::Id, RoseTree<T>>,
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
        tree.tree(&path[..path.len() - 1])
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
        tree.tree_mut(&path[..path.len() - 1])
    }

    /// Map the `RoseTree` from elem type T to U.
    ///
    /// The given function is called for every element in the tree.
    pub(crate) fn _map<U>(self, f: &mut impl FnMut(T) -> U) -> RoseTree<U> {
        let Self { elem, nested } = self;
        let elem = f(elem);
        let nested = nested.into_iter().map(|(k, r)| (k, r._map(f))).collect();
        RoseTree { elem, nested }
    }

    /// Map the `RoseTree` by reference to a `RoseTree` with a new elem type.
    ///
    /// This is useful if the resulting tree requires holding references to the
    /// original tree.
    pub(crate) fn map_ref<'a, U>(&'a self, f: &mut impl FnMut(&'a T) -> U) -> RoseTree<U> {
        let Self { elem, nested } = self;
        let elem = f(elem);
        let nested = nested
            .into_iter()
            .map(|(&k, r)| (k, r.map_ref(f)))
            .collect();
        RoseTree { elem, nested }
    }

    /// Visit all nodes in depth-first order where the given `path` is the
    /// path to `self` from the root.
    pub(crate) fn visit(&self, path: &[node::Id], f: &mut impl FnMut(&[node::Id], &T)) {
        f(path, &self.elem);
        let mut path = path.to_vec();
        for (&id, tree) in &self.nested {
            path.push(id);
            tree.visit(&path, f);
            path.pop();
        }
    }
}
