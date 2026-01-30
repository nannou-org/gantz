use crate::graph::{Graph, GraphNode};
use crate::node::Node;
use bevy_gantz::{BuiltinNodes, Builtins, Registry, Views};
use gantz_ca as ca;
use std::collections::{BTreeMap, HashMap};

/// Reference-based environment for VM operations.
///
/// This is a concrete type (not generic over N) to avoid trait bound cycles.
/// Constructed on-demand from borrowed Bevy resources.
pub struct Environment<'a> {
    /// The registry of all graphs, commits and names.
    pub registry: &'a ca::Registry<Graph>,
    /// Views (layout + camera) for all known commits.
    pub views: &'a Views,
    /// Builtins (primitive nodes).
    pub builtins: &'a dyn Builtins<Node = Box<dyn crate::node::Node>>,
}

impl<'a> Environment<'a> {
    /// Create a new environment from borrowed resources.
    pub fn new(
        registry: &'a Registry<Box<dyn crate::node::Node>>,
        views: &'a Views,
        builtins: &'a BuiltinNodes<Box<dyn crate::node::Node>>,
    ) -> Self {
        Self {
            registry: &registry.0,
            views,
            builtins: &*builtins.0,
        }
    }
}

// Provide the `NodeTypeRegistry` implementation required by `gantz_egui`.
impl gantz_egui::widget::gantz::NodeTypeRegistry for Environment<'_> {
    type Node = Box<dyn crate::node::Node>;

    fn node_types(&self) -> impl Iterator<Item = &str> {
        let mut types = vec![];
        types.extend(self.builtins.names());
        types.extend(self.registry.names().keys().map(|s| &s[..]));
        types.sort();
        types.into_iter()
    }

    fn new_node(&self, node_type: &str) -> Option<Self::Node> {
        self.registry
            .names()
            .get(node_type)
            .map(|commit_ca| {
                // Store CommitAddr directly (converted to ContentAddr).
                let ref_ = gantz_core::node::Ref::new((*commit_ca).into());
                let named = gantz_egui::node::NamedRef::new(node_type.to_string(), ref_);
                Box::new(named) as Box<_>
            })
            .or_else(|| self.builtins.create(node_type))
    }
}

// Provide the `NodeRegistry` implementation required by `gantz_core::node::Ref`.
impl gantz_core::node::ref_::NodeRegistry for Environment<'_> {
    type Node = dyn gantz_core::Node<Self>;
    fn node(&self, ca: &ca::ContentAddr) -> Option<&Self::Node> {
        // Try commit lookup (for graph refs stored as CommitAddr).
        let commit_ca = ca::CommitAddr::from(*ca);
        if let Some(graph) = self.registry.commit_graph_ref(&commit_ca) {
            return Some(graph as &dyn gantz_core::Node<Self>);
        }
        // Fall back to builtin lookup.
        self.builtins
            .instance(ca)
            .map(|n| &**n as &dyn gantz_core::Node<Self>)
    }
}

// Provide the `GraphRegistry` implementation required by the `GraphSelect` widget.
impl gantz_egui::widget::graph_select::GraphRegistry for Environment<'_> {
    fn commits(&self) -> Vec<(&ca::CommitAddr, &ca::Commit)> {
        // Sort commits by newest to oldest.
        let mut commits: Vec<_> = self.registry.commits().iter().collect();
        commits.sort_by(|(_, a), (_, b)| b.timestamp.cmp(&a.timestamp));
        commits
    }

    fn names(&self) -> &BTreeMap<String, ca::CommitAddr> {
        self.registry.names()
    }
}

// Provide the `NameRegistry` implementation required by `gantz_egui::node::NamedRef`.
impl gantz_egui::node::NameRegistry for Environment<'_> {
    fn name_ca(&self, name: &str) -> Option<ca::ContentAddr> {
        // Check registry names first (graphs shadow builtins).
        // Return CommitAddr (as ContentAddr) for graph nodes.
        if let Some(commit_ca) = self.registry.names().get(name) {
            return Some((*commit_ca).into());
        }
        // Then check builtin names.
        self.builtins.content_addr(name)
    }
}

// Provide the `FnNodeNames` implementation required by `Fn<NamedRef>` UI.
impl gantz_egui::node::FnNodeNames for Environment<'_> {
    fn fn_node_names(&self) -> Vec<String> {
        use gantz_core::node::ref_::NodeRegistry;
        use gantz_egui::node::NameRegistry;

        // Collect all names (builtins + registry names).
        let builtin_names = self
            .builtins
            .names()
            .into_iter()
            .filter_map(|name| self.builtins.content_addr(name).map(|_| name.to_string()));
        let registry_names = self.registry.names().keys().cloned();
        let all_names = builtin_names.chain(registry_names);

        // Filter to Fn-compatible nodes (stateless, branchless, 1 output).
        let mut names: Vec<_> = all_names
            .filter(|name| {
                self.name_ca(name)
                    .and_then(|ca| self.node(&ca))
                    .map(|n| {
                        !n.stateful(self) && n.branches(self).is_empty() && n.n_outputs(self) == 1
                    })
                    .unwrap_or(false)
            })
            .collect();

        names.sort();
        names
    }
}

// ----------------------------------------------------------------------------
// AppBuiltins
// ----------------------------------------------------------------------------

/// Constructors for all builtin nodes.
type Primitives = BTreeMap<String, Box<dyn Send + Sync + Fn() -> Box<dyn Node>>>;

/// Instantiated builtin nodes keyed by their content address.
type PrimitiveInstances = HashMap<ca::ContentAddr, Box<dyn Node>>;

/// Mapping from builtin content addresses to their names.
type PrimitiveNames = HashMap<ca::ContentAddr, String>;

/// The set of all known node types accessible to gantz.
fn primitives() -> Primitives {
    let mut p = Primitives::default();
    register_primitive(&mut p, "add", || {
        Box::new(gantz_std::ops::Add::default()) as Box<_>
    });
    register_primitive(&mut p, "apply", || {
        Box::new(gantz_core::node::Apply::default()) as Box<_>
    });
    register_primitive(&mut p, "bang", || {
        Box::new(gantz_std::Bang::default()) as Box<_>
    });
    register_primitive(&mut p, "comment", || {
        Box::new(gantz_egui::node::Comment::default()) as Box<_>
    });
    register_primitive(&mut p, "expr", || {
        Box::new(gantz_core::node::Expr::new("()").unwrap()) as Box<_>
    });
    // Compute Identity's CA for the default Fn<NamedRef>.
    let identity_ca = ca::content_addr(&gantz_core::node::Identity);
    register_primitive(&mut p, "fn", move || {
        let named_ref = gantz_egui::node::NamedRef::new(
            gantz_core::node::IDENTITY_NAME.to_string(),
            gantz_core::node::Ref::new(identity_ca),
        );
        Box::new(gantz_core::node::Fn::new(named_ref)) as Box<_>
    });
    register_primitive(&mut p, "graph", || Box::new(GraphNode::default()) as Box<_>);
    register_primitive(&mut p, gantz_core::node::IDENTITY_NAME, || {
        Box::new(gantz_core::node::Identity::default()) as Box<_>
    });
    register_primitive(&mut p, "inlet", || {
        Box::new(gantz_core::node::graph::Inlet::default()) as Box<_>
    });
    register_primitive(&mut p, "inspect", || {
        Box::new(gantz_egui::node::Inspect::default()) as Box<_>
    });
    register_primitive(&mut p, "log", || {
        Box::new(gantz_std::Log::default()) as Box<_>
    });
    register_primitive(&mut p, "number", || {
        Box::new(gantz_std::Number::default()) as Box<_>
    });
    register_primitive(&mut p, "outlet", || {
        Box::new(gantz_core::node::graph::Outlet::default()) as Box<_>
    });
    p
}

fn register_primitive(
    primitives: &mut Primitives,
    name: impl Into<String>,
    new: impl 'static + Send + Sync + Fn() -> Box<dyn Node>,
) -> Option<Box<dyn Send + Sync + Fn() -> Box<dyn Node>>> {
    primitives.insert(name.into(), Box::new(new) as Box<_>)
}

fn primitive_instances_and_names(primitives: &Primitives) -> (PrimitiveInstances, PrimitiveNames) {
    let mut instances = PrimitiveInstances::default();
    let mut names = PrimitiveNames::default();
    for (name, ctor) in primitives.iter() {
        let node = ctor();
        let content_addr = ca::content_addr(&node);
        instances.insert(content_addr, node);
        names.insert(content_addr, name.clone());
    }
    (instances, names)
}

/// Application-specific builtins implementation.
pub struct AppBuiltins {
    /// Constructors for all builtin nodes.
    constructors: Primitives,
    /// Instantiated builtin nodes keyed by their content address.
    instances: PrimitiveInstances,
    /// Mapping from content addresses to names.
    names: PrimitiveNames,
}

impl AppBuiltins {
    pub fn new() -> Self {
        let constructors = primitives();
        let (instances, names) = primitive_instances_and_names(&constructors);
        Self {
            constructors,
            instances,
            names,
        }
    }
}

impl Default for AppBuiltins {
    fn default() -> Self {
        Self::new()
    }
}

impl Builtins for AppBuiltins {
    type Node = Box<dyn Node>;

    fn names(&self) -> Vec<&str> {
        self.constructors.keys().map(|s| s.as_str()).collect()
    }

    fn create(&self, name: &str) -> Option<Self::Node> {
        self.constructors.get(name).map(|f| f())
    }

    fn instance(&self, ca: &ca::ContentAddr) -> Option<&Self::Node> {
        self.instances.get(ca)
    }

    fn name(&self, ca: &ca::ContentAddr) -> Option<&str> {
        self.names.get(ca).map(|s| s.as_str())
    }

    fn content_addr(&self, name: &str) -> Option<ca::ContentAddr> {
        self.names
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(ca, _)| *ca)
    }
}
