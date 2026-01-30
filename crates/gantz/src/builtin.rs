//! Application-specific builtin nodes.

use crate::{GraphNode, node::Node};
use gantz_ca as ca;
use std::collections::{BTreeMap, HashMap};

/// Constructors for all builtin nodes.
type Primitives = BTreeMap<String, Box<dyn Send + Sync + Fn() -> Box<dyn Node>>>;

/// Instantiated builtin nodes keyed by their content address.
type PrimitiveInstances = HashMap<ca::ContentAddr, Box<dyn Node>>;

/// Mapping from builtin content addresses to their names.
type PrimitiveNames = HashMap<ca::ContentAddr, String>;

/// The set of all known node types accessible to gantz.
pub struct Builtins {
    /// Constructors for all builtin nodes.
    constructors: Primitives,
    /// Instantiated builtin nodes keyed by their content address.
    instances: PrimitiveInstances,
    /// Mapping from content addresses to names.
    names: PrimitiveNames,
}

impl Builtins {
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

impl Default for Builtins {
    fn default() -> Self {
        Self::new()
    }
}

impl bevy_gantz::Builtins for Builtins {
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
