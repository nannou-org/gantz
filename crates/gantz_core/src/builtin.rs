//! Builtin nodes as plain data: composable specs and the composed palette.
//!
//! Builtins are nodes that are always available and not stored in the
//! registry. They typically include primitive operations like arithmetic,
//! control flow, etc. Each domain crate exports its builtin node set as a
//! plain `fn builtins() -> Vec<Builtin>`; applications compose the domain
//! lists into a [`Builtins`].

use crate::data;
use gantz_ca::{ContentAddr, NodeData};
use std::collections::{BTreeMap, HashMap};

/// One palette entry: a builtin node's name and its erased data form.
#[derive(Clone, Debug)]
pub struct Builtin {
    /// The unique name identifying the builtin (e.g. `"expr"`).
    pub name: &'static str,
    /// The builtin's default instance, erased to its canonical data form.
    pub node: NodeData,
}

/// A composed builtin palette: name <-> erased node data, as plain data.
///
/// The erased [`NodeData`] content address is a builtin's one network-wide
/// address - the same scheme registry graphs use.
#[derive(Clone, Debug, Default)]
pub struct Builtins {
    /// Erased builtin nodes keyed by name.
    by_name: BTreeMap<&'static str, NodeData>,
    /// Erased content address by name.
    addr_by_name: BTreeMap<&'static str, ContentAddr>,
    /// The reverse index: name by erased content address.
    name_by_addr: HashMap<ContentAddr, &'static str>,
}

impl Builtin {
    /// A new builtin spec: erase the given default instance under its
    /// declared [`NodeTag`](gantz_nodetag::NodeTag).
    ///
    /// Panics if erasure fails - a builtin that cannot erase is a node-set
    /// composition error, caught at startup or in tests.
    pub fn new<T>(name: &'static str, node: &T) -> Self
    where
        T: gantz_nodetag::NodeTag + serde::Serialize + crate::Node,
    {
        let node = data::erase_node_typed(node)
            .unwrap_or_else(|e| panic!("builtin `{name}` failed to erase: {e}"));
        Self { name, node }
    }
}

impl Builtins {
    /// Compose a palette from the given specs.
    ///
    /// Panics on duplicate names AND duplicate addresses (two names erasing
    /// to identical [`NodeData`] would shadow each other in the reverse
    /// index) - both indicate a composition error.
    pub fn from_specs(specs: impl IntoIterator<Item = Builtin>) -> Self {
        let mut by_name = BTreeMap::new();
        let mut addr_by_name = BTreeMap::new();
        let mut name_by_addr = HashMap::new();
        for spec in specs {
            let addr = spec.node.content_addr();
            if let Some(prev) = name_by_addr.insert(addr, spec.name) {
                panic!(
                    "builtins `{prev}` and `{}` share the content address {addr}",
                    spec.name,
                );
            }
            addr_by_name.insert(spec.name, addr);
            if by_name.insert(spec.name, spec.node).is_some() {
                panic!("duplicate builtin name: `{}`", spec.name);
            }
        }
        Self {
            by_name,
            addr_by_name,
            name_by_addr,
        }
    }

    /// All builtin names, in name order.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_name.keys().copied()
    }

    /// The erased node data of the builtin with the given name.
    pub fn node_data(&self, name: &str) -> Option<&NodeData> {
        self.by_name.get(name)
    }

    /// The erased node data of the builtin with the given content address.
    pub fn get(&self, ca: &ContentAddr) -> Option<&NodeData> {
        self.name(ca).and_then(|name| self.by_name.get(name))
    }

    /// The name of the builtin with the given content address.
    pub fn name(&self, ca: &ContentAddr) -> Option<&'static str> {
        self.name_by_addr.get(ca).copied()
    }

    /// The content address of the builtin with the given name.
    pub fn content_addr(&self, name: &str) -> Option<ContentAddr> {
        self.addr_by_name.get(name).copied()
    }
}
