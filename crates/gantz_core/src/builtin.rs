//! Builtin node provider trait and composable builtin specs.

use gantz_ca::ContentAddr;
use std::collections::{BTreeMap, HashMap};

/// Trait for providing builtin (hard-coded) nodes.
///
/// Builtins are nodes that are always available and not stored in the registry.
/// They typically include primitive operations like arithmetic, control flow, etc.
///
/// This trait is object-safe when used as `dyn Builtins<Node = N>`.
pub trait Builtins: Send + Sync {
    /// The node type produced by this builtins provider.
    type Node: 'static + Send + Sync;

    /// Get all builtin node names.
    fn names(&self) -> Vec<&str>;

    /// Create a new instance of a builtin node by name.
    fn create(&self, name: &str) -> Option<Self::Node>;

    /// Get a builtin node instance by content address.
    fn instance(&self, ca: &ContentAddr) -> Option<&Self::Node>;

    /// Get the name of a builtin by content address.
    fn name(&self, ca: &ContentAddr) -> Option<&str>;

    /// Get content address by name.
    fn content_addr(&self, name: &str) -> Option<ContentAddr>;

    /// Get the name of the demo graph associated with a builtin, if any.
    fn demo_graph(&self, name: &str) -> Option<&str> {
        let _ = name;
        None
    }
}

/// A node-set type that can absorb a node of type `T`.
///
/// Node-set types (e.g. an app's `Box<dyn Node>`) implement this once via a
/// blanket impl, allowing domain crates to provide [`Builtin`] spec lists
/// generic over any compatible node set.
pub trait FromNode<T> {
    /// Convert a node of type `T` into the node-set type.
    fn from_node(node: T) -> Self;
}

/// One palette entry: a builtin node's name and constructor.
///
/// Domain crates export their builtin node set as a plain
/// `fn builtins<N>() -> Vec<Builtin<N>>` where `N` is bound by [`FromNode`]
/// for each of the domain's node types. Apps compose the domain lists into a
/// [`BuiltinSet`].
pub struct Builtin<N> {
    /// The unique name identifying the builtin (e.g. `"expr"`).
    pub name: &'static str,
    /// Constructs a fresh default instance of the node.
    pub new: Box<dyn Fn() -> N + Send + Sync>,
    /// The name of the demo graph associated with this builtin, if any.
    pub demo_graph: Option<&'static str>,
}

/// A generic [`Builtins`] implementation over a composed list of [`Builtin`]
/// specs.
pub struct BuiltinSet<N> {
    /// Builtin specs keyed by name.
    builtins: BTreeMap<&'static str, Builtin<N>>,
    /// Instantiated builtin nodes keyed by their content address.
    instances: HashMap<ContentAddr, N>,
    /// Mapping from content addresses to names.
    names: HashMap<ContentAddr, &'static str>,
}

impl<N> Builtin<N> {
    /// A new builtin spec with no demo graph association.
    pub fn new(name: &'static str, new: impl Fn() -> N + Send + Sync + 'static) -> Self {
        Self {
            name,
            new: Box::new(new),
            demo_graph: None,
        }
    }
}

impl<N> BuiltinSet<N>
where
    N: gantz_ca::CaHash + Send + Sync + 'static,
{
    /// Compose a set from the given specs.
    ///
    /// Instantiates each builtin once to index it by content address.
    ///
    /// Panics on duplicate names, as duplicates indicate a composition error.
    pub fn from_specs(specs: impl IntoIterator<Item = Builtin<N>>) -> Self {
        let mut builtins = BTreeMap::new();
        let mut instances = HashMap::new();
        let mut names = HashMap::new();
        for spec in specs {
            let node = (spec.new)();
            let ca = gantz_ca::content_addr(&node);
            instances.insert(ca, node);
            names.insert(ca, spec.name);
            if let Some(prev) = builtins.insert(spec.name, spec) {
                panic!("duplicate builtin name: {:?}", prev.name);
            }
        }
        Self {
            builtins,
            instances,
            names,
        }
    }
}

impl<N: 'static + Send + Sync> Builtins for BuiltinSet<N> {
    type Node = N;

    fn names(&self) -> Vec<&str> {
        self.builtins.keys().copied().collect()
    }

    fn create(&self, name: &str) -> Option<Self::Node> {
        self.builtins.get(name).map(|b| (b.new)())
    }

    fn instance(&self, ca: &ContentAddr) -> Option<&Self::Node> {
        self.instances.get(ca)
    }

    fn name(&self, ca: &ContentAddr) -> Option<&str> {
        self.names.get(ca).copied()
    }

    fn content_addr(&self, name: &str) -> Option<ContentAddr> {
        self.names
            .iter()
            .find(|(_, n)| **n == name)
            .map(|(ca, _)| *ca)
    }

    fn demo_graph(&self, name: &str) -> Option<&str> {
        self.builtins.get(name).and_then(|b| b.demo_graph)
    }
}
