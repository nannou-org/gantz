use super::{Deserialize, Serialize};
use crate::{
    ROOT_STATE,
    node::{self, Node},
};
use petgraph::visit::{IntoNodeReferences, NodeIndexable, NodeRef};
use steel::{
    SteelErr, SteelVal,
    gc::Gc,
    parser::ast::ExprKind,
    rerrs::ErrorKind,
    rvals::{FromSteelVal, IntoSteelVal, SteelHashMap},
    steel_vm::engine::Engine,
};

/// Types that may be used as state for a [`Node`].
// FIXME: Does `derive(Steel)` already do all this? Is there a trait for this?
// TODO: If not, we should add a `derive` for this and its `impl`.
pub trait NodeState: Default + FromSteelVal + IntoSteelVal {
    /// The name of the state type.
    const NAME: &str;
    /// Register the set of functions required by nodes for working with this
    /// state.
    fn register_fns(vm: &mut Engine);
    /// Provided method that automatically registers the type followed by a call
    /// to `register_fns`.
    fn register(vm: &mut Engine) {
        vm.register_type::<Self>(Self::NAME);
        Self::register_fns(vm);
    }
}

/// A trait implemented for all **Node** types allowing to add some state accessible to its
/// expression. This is particularly useful for adding state to **Expr** nodes.
pub trait WithStateType: Node + Sized {
    /// Consume `self` and return a `Node` that has state of type `state_type`.
    fn with_state_type<S: NodeState>(self) -> State<Self, S> {
        State::<Self, S>::new(self)
    }
}

/// A wrapper around a **Node** that adds some persistent state.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct State<N, S> {
    /// The node being wrapped with state.
    pub node: N,
    /// The type of state used by the node.
    pub state: core::marker::PhantomData<S>,
}

impl<N, S> State<N, S> {
    /// Given some node, return a **State** node enabling access to state of the
    /// given type.
    pub fn new(node: N) -> Self
    where
        N: Node,
        S: NodeState,
    {
        State {
            node,
            state: core::marker::PhantomData,
        }
    }
}

fn default_node_state_steel_val<S: NodeState>() -> SteelVal {
    S::default()
        .into_steelval()
        .expect("default `NodeState` to `SteelVal` conversion should never fail")
}

impl<N> WithStateType for N
where
    N: Node,
{
    fn with_state_type<S: NodeState>(self) -> State<Self, S> {
        State::<Self, S>::new(self)
    }
}

impl<N, S> Node for State<N, S>
where
    N: Node,
    S: NodeState,
{
    fn n_inputs(&self) -> usize {
        self.node.n_inputs()
    }

    fn n_outputs(&self) -> usize {
        self.node.n_outputs()
    }

    fn expr(&self, inputs: &[Option<ExprKind>]) -> ExprKind {
        self.node.expr(inputs)
    }

    fn push_eval(&self) -> Option<node::EvalFn> {
        self.node.push_eval()
    }

    fn pull_eval(&self) -> Option<node::EvalFn> {
        self.node.pull_eval()
    }

    fn register_state(&self, path: &[node::Id], vm: &mut Engine) {
        S::register(vm);
        let val = default_node_state_steel_val::<S>();
        register(vm, path, val).unwrap();
    }
}

/// Register all node state types within the given VM.
pub fn register_graph<G>(g: G, vm: &mut Engine)
where
    G: IntoNodeReferences + NodeIndexable,
    G::NodeWeight: Node,
{
    for n in g.node_references() {
        let id = g.to_index(n.id());
        n.weight().register_state(&[id], vm);
    }
}

/// Sets the given node's state to the given value.
pub fn register_value(vm: &mut Engine, node_path: &[usize], val: SteelVal) -> Result<(), SteelErr> {
    let SteelVal::HashMapV(mut root_state) = vm.extract_value(ROOT_STATE)? else {
        return Err(SteelErr::new(
            ErrorKind::Generic,
            "`ROOT_STATE` was not a hashmap".to_string(),
        ));
    };

    // Traverse the state tree to update the node value at the given path.
    fn update_value(
        graph_state: &mut SteelHashMap,
        node_path: &[usize],
        val: SteelVal,
    ) -> Result<(), SteelErr> {
        match node_path {
            &[] => Err(SteelErr::new(ErrorKind::Generic, "empty node path".into())),
            &[node_id] => {
                let id = node_id.try_into().expect("node_id out of range");
                let key = SteelVal::IntV(id);
                *graph_state = Gc::new(graph_state.update(key, val)).into();
                Ok(())
            }
            &[graph_id, ..] => {
                let id = graph_id.try_into().expect("node_id out of range");
                let key = SteelVal::IntV(id);
                let update = |opt: Option<SteelVal>| {
                    let Some(SteelVal::HashMapV(mut state)) = opt else {
                        panic!("graph state was not a hashmap");
                    };
                    update_value(&mut state, &node_path[1..], val).expect("failed to update value");
                    Some(SteelVal::HashMapV(state))
                };
                *graph_state = Gc::new(graph_state.alter(update, key)).into();
                Ok(())
            }
        }
    }

    update_value(&mut root_state, node_path, val)?;
    vm.register_value(ROOT_STATE, SteelVal::HashMapV(root_state));
    Ok(())
}

/// Sets the given node's state to the given value.
// TODO: Change `node_id: usize` to `node_path: &[usize]` to support nesting.
pub fn register<S: IntoSteelVal>(
    vm: &mut Engine,
    node_path: &[usize],
    val: S,
) -> Result<(), SteelErr> {
    register_value(vm, node_path, val.into_steelval()?)
}

/// Extract the value for the node with the given ID.
// TODO: Change `node_id: usize` to `node_path: &[usize]` to support nesting.
pub fn extract_value(vm: &Engine, node_id: usize) -> Result<Option<SteelVal>, SteelErr> {
    let SteelVal::HashMapV(graph_state) = vm.extract_value(ROOT_STATE)? else {
        return Ok(None);
    };
    let ix = isize::try_from(node_id).expect("node ID out of range");
    let key = &SteelVal::IntV(ix);
    Ok(graph_state.get(key).cloned())
}

/// Extract the value for the node with the given ID.
// TODO: Change `node_id: usize` to `node_path: &[usize]` to support nesting.
pub fn extract<S: FromSteelVal>(vm: &Engine, node_id: usize) -> Result<Option<S>, SteelErr> {
    let Some(val) = extract_value(vm, node_id)? else {
        return Ok(None);
    };
    S::from_steelval(&val).map(Some)
}
