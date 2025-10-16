use super::{Deserialize, Serialize};
use crate::{
    ROOT_STATE,
    node::{self, Node},
};
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
pub trait WithStateType<Env>: Node<Env> + Sized {
    /// Consume `self` and return a `Node` that has state of type `state_type`.
    fn with_state_type<S: NodeState>(self) -> State<Env, Self, S> {
        State::<Env, Self, S>::new(self)
    }
}

/// A wrapper around a **Node** that adds some persistent state.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct State<Env, N, S> {
    pub env: core::marker::PhantomData<Env>,
    /// The node being wrapped with state.
    pub node: N,
    /// The type of state used by the node.
    pub state: core::marker::PhantomData<S>,
}

impl<Env, N, S> State<Env, N, S> {
    /// Given some node, return a **State** node enabling access to state of the
    /// given type.
    pub fn new(node: N) -> Self
    where
        N: Node<Env>,
        S: NodeState,
    {
        State {
            env: core::marker::PhantomData,
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

impl<Env, N> WithStateType<Env> for N
where
    N: Node<Env>,
{
    fn with_state_type<S: NodeState>(self) -> State<Env, Self, S> {
        State::<Env, Self, S>::new(self)
    }
}

impl<Env, N, S> Node<Env> for State<Env, N, S>
where
    N: Node<Env>,
    S: NodeState,
{
    fn n_inputs(&self, env: &Env) -> usize {
        self.node.n_inputs(env)
    }

    fn n_outputs(&self, env: &Env) -> usize {
        self.node.n_outputs(env)
    }

    fn branches(&self, env: &Env) -> Vec<node::EvalConf> {
        self.node.branches(env)
    }

    fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
        self.node.expr(ctx)
    }

    fn push_eval(&self, env: &Env) -> Vec<node::EvalConf> {
        self.node.push_eval(env)
    }

    fn pull_eval(&self, env: &Env) -> Vec<node::EvalConf> {
        self.node.pull_eval(env)
    }

    fn inlet(&self) -> bool {
        self.node.inlet()
    }

    fn outlet(&self) -> bool {
        self.node.outlet()
    }

    fn stateful(&self) -> bool {
        true
    }

    fn register(&self, path: &[node::Id], vm: &mut Engine) {
        S::register(vm);
        let val = default_node_state_steel_val::<S>();
        update(vm, path, val).unwrap();
    }
}

/// Sets the given node's state to the given value.
pub fn update_value(vm: &mut Engine, node_path: &[usize], val: SteelVal) -> Result<(), SteelErr> {
    let SteelVal::HashMapV(mut root_state) = vm.extract_value(ROOT_STATE)? else {
        return Err(SteelErr::new(
            ErrorKind::Generic,
            "`ROOT_STATE` was not a hashmap".to_string(),
        ));
    };

    // Traverse the state tree to update the node value at the given path.
    fn update_hashmap_value(
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
                    update_hashmap_value(&mut state, &node_path[1..], val)
                        .expect("failed to update value");
                    Some(SteelVal::HashMapV(state))
                };
                *graph_state = Gc::new(graph_state.alter(update, key)).into();
                Ok(())
            }
        }
    }

    update_hashmap_value(&mut root_state, node_path, val)?;
    vm.update_value(ROOT_STATE, SteelVal::HashMapV(root_state));
    Ok(())
}

/// Sets the given node's state to the given value.
// TODO: Change `node_id: usize` to `node_path: &[usize]` to support nesting.
pub fn update<S: IntoSteelVal>(
    vm: &mut Engine,
    node_path: &[usize],
    val: S,
) -> Result<(), SteelErr> {
    update_value(vm, node_path, val.into_steelval()?)
}

/// Extract the value for the node with the given ID.
pub fn extract_value(vm: &Engine, node_path: &[usize]) -> Result<Option<SteelVal>, SteelErr> {
    let SteelVal::HashMapV(root_state) = vm.extract_value(ROOT_STATE)? else {
        return Err(SteelErr::new(
            ErrorKind::Generic,
            "`ROOT_STATE` was not a hashmap".to_string(),
        ));
    };

    // Traverse the state tree to extract the node value at the given path.
    fn extract_hashmap_value(
        graph_state: &SteelHashMap,
        node_path: &[usize],
    ) -> Result<Option<SteelVal>, SteelErr> {
        match node_path {
            &[] => Err(SteelErr::new(ErrorKind::Generic, "empty node path".into())),
            &[node_id] => {
                let id = node_id.try_into().expect("node_id out of range");
                let key = SteelVal::IntV(id);
                Ok(graph_state.get(&key).cloned())
            }
            &[graph_id, ..] => {
                let id = graph_id.try_into().expect("node_id out of range");
                let key = SteelVal::IntV(id);
                let Some(SteelVal::HashMapV(state)) = graph_state.get(&key) else {
                    return Ok(None);
                };
                extract_hashmap_value(state, &node_path[1..])
            }
        }
    }

    extract_hashmap_value(&root_state, node_path)
}

/// Extract the value for the node with the given ID.
pub fn extract<S: FromSteelVal>(vm: &Engine, node_path: &[usize]) -> Result<Option<S>, SteelErr> {
    let Some(val) = extract_value(vm, node_path)? else {
        return Ok(None);
    };
    S::from_steelval(&val).map(Some)
}
