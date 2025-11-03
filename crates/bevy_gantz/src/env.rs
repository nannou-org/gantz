//! A gantz `Environment` to be provided to nodes in a bevy + gantz
//! implementation.


/// The type used to track mappings between node names, content addresses and
/// graphs. Also provides access to the node registry. This can be thought of as
/// a shared immutable input to all nodes.
struct Environment {
    /// Constructors for all primitive nodes.
    primitives: Primitives,
    /// The registry of all nodes composed from other nodes.
    registry: NodeTypeRegistry,
}

/// The registry for all named graphs, i.e. nodes composed from other nodes.
#[derive(Default, Deserialize, Serialize)]
struct NodeTypeRegistry {
    /// A mapping from content addresses to graphs.
    graphs: HashMap<ContentAddr, Graph>,
    /// A mapping from names to graph content addresses.
    names: BTreeMap<String, ContentAddr>,
}

/// Constructors for all primitive nodes.
type Primitives = BTreeMap<String, Box<dyn Fn() -> Box<dyn Node>>>;

// Provide the `NodeTypeRegistry` implementation required by `gantz_egui`.
impl gantz_egui::widget::gantz::NodeTypeRegistry for Environment {
    type Node = Box<dyn Node>;

    fn node_types(&self) -> impl Iterator<Item = &str> {
        let mut types = vec![];
        types.extend(self.primitives.keys().map(|s| &s[..]));
        types.extend(self.registry.names.keys().map(|s| &s[..]));
        types.sort();
        types.into_iter()
    }

    fn new_node(&self, node_type: &str) -> Option<Self::Node> {
        self.registry
            .names
            .get(node_type)
            .map(|&ca| {
                let named = gantz_egui::node::NamedGraph::new(node_type.to_string(), ca);
                Box::new(named) as Box<_>
            })
            .or_else(|| self.primitives.get(node_type).map(|f| (f)()))
    }
}

// Provide the `GraphRegistry` implementation required by `gantz_egui`.
impl gantz_egui::node::graph::GraphRegistry for Environment {
    type Node = Box<dyn Node>;
    fn graph(&self, ca: ContentAddr) -> Option<&gantz_core::node::graph::Graph<Self::Node>> {
        self.registry.graphs.get(&ca)
    }
}

// Provide the `GraphRegistry` implementation required by the `GraphSelect` widget.
impl gantz_egui::widget::graph_select::GraphRegistry for Environment {
    fn addrs(&self) -> Vec<ContentAddr> {
        let mut vec: Vec<_> = self.registry.graphs.keys().copied().collect();
        vec.sort();
        vec
    }

    fn names(&self) -> &BTreeMap<String, ContentAddr> {
        &self.registry.names
    }
}

/// The set of all known node types accessible to gantz.
fn primitives() -> Primitives {
    let mut p = Primitives::default();
    register_primitive(&mut p, "add", || {
        Box::new(gantz_std::ops::Add::default()) as Box<_>
    });
    register_primitive(&mut p, "bang", || {
        Box::new(gantz_std::Bang::default()) as Box<_>
    });
    register_primitive(&mut p, "expr", || {
        Box::new(gantz_core::node::Expr::new("()").unwrap()) as Box<_>
    });
    register_primitive(&mut p, "graph", || Box::new(GraphNode::default()) as Box<_>);
    register_primitive(&mut p, "inlet", || {
        Box::new(gantz_core::node::graph::Inlet::default()) as Box<_>
    });
    register_primitive(&mut p, "outlet", || {
        Box::new(gantz_core::node::graph::Outlet::default()) as Box<_>
    });
    register_primitive(&mut p, "log", || {
        Box::new(gantz_std::Log::default()) as Box<_>
    });
    register_primitive(&mut p, "number", || {
        Box::new(gantz_std::Number::default()) as Box<_>
    });
    p
}

fn register_primitive(
    primitives: &mut Primitives,
    name: impl Into<String>,
    new: impl 'static + Fn() -> Box<dyn Node>,
) -> Option<Box<dyn Fn() -> Box<dyn Node>>> {
    primitives.insert(name.into(), Box::new(new) as Box<_>)
}
