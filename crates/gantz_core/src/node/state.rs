use super::{Deserialize, Serialize};
use crate::{
    ROOT_STATE,
    node::{self, Node},
};
use gantz_ca::CaHash;
use steel::{
    SteelErr, SteelVal,
    gc::Gc,
    rerrs::ErrorKind,
    rvals::{FromSteelVal, IntoSteelVal, SteelHashMap},
    steel_vm::engine::Engine,
};

/// A wrapper around a **Node** that adds some persistent state.
#[derive(Clone, Debug, Deserialize, Serialize, CaHash)]
#[cahash("gantz.state")]
pub struct State<N, S> {
    /// The node being wrapped with state.
    pub node: N,
    /// The type of state used by the node.
    #[cahash(skip)]
    pub state: core::marker::PhantomData<S>,
}

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

impl<N: Node> WithStateType for N {
    fn with_state_type<S: NodeState>(self) -> State<Self, S> {
        State::<Self, S>::new(self)
    }
}

impl<N, S> Node for State<N, S>
where
    N: Node,
    S: NodeState,
{
    fn n_inputs(&self, ctx: node::MetaCtx) -> usize {
        self.node.n_inputs(ctx)
    }

    fn n_outputs(&self, ctx: node::MetaCtx) -> usize {
        self.node.n_outputs(ctx)
    }

    fn branches(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        self.node.branches(ctx)
    }

    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        self.node.expr(ctx)
    }

    fn push_eval(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        self.node.push_eval(ctx)
    }

    fn pull_eval(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        self.node.pull_eval(ctx)
    }

    fn inlet(&self, ctx: node::MetaCtx) -> bool {
        self.node.inlet(ctx)
    }

    fn outlet(&self, ctx: node::MetaCtx) -> bool {
        self.node.outlet(ctx)
    }

    fn stateful(&self, _ctx: node::MetaCtx) -> bool {
        true
    }

    fn register(&self, ctx: node::RegCtx<'_, '_>) {
        let (get_node, path, vm) = ctx.into_parts();
        S::register(vm);
        // Only initialize state if not already present.
        if extract_value(vm, path).ok().flatten().is_none() {
            let val = default_node_state_steel_val::<S>();
            update(vm, path, val).unwrap();
        }
        // Register the inner node.
        self.node.register(node::RegCtx::new(get_node, path, vm));
    }

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        self.node.required_addrs()
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
                    // Lazily initialize empty hashmap if not present.
                    let mut state = match opt {
                        Some(SteelVal::HashMapV(state)) => state,
                        None => Gc::new(steel::HashMap::new()).into(),
                        Some(_) => panic!("graph state was not a hashmap"),
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

/// Remove the state value at the given node path.
///
/// For a path like `[5]`, removes key `5` from `%root-state`.
/// For a path like `[5, 3]`, traverses into `%root-state[5]` and removes key `3`.
///
/// No-op if the key doesn't exist or if `ROOT_STATE` hasn't been initialized.
pub fn remove_value(vm: &mut Engine, node_path: &[usize]) -> Result<(), SteelErr> {
    let root_val = match vm.extract_value(ROOT_STATE) {
        Ok(val) => val,
        // No root state initialized yet, nothing to remove.
        Err(_) => return Ok(()),
    };
    let SteelVal::HashMapV(mut root_state) = root_val else {
        return Err(SteelErr::new(
            ErrorKind::Generic,
            "`ROOT_STATE` was not a hashmap".to_string(),
        ));
    };

    fn remove_hashmap_value(
        graph_state: &mut SteelHashMap,
        node_path: &[usize],
    ) -> Result<(), SteelErr> {
        match node_path {
            &[] => Err(SteelErr::new(ErrorKind::Generic, "empty node path".into())),
            &[node_id] => {
                let id = node_id.try_into().expect("node_id out of range");
                let key = SteelVal::IntV(id);
                *graph_state = Gc::new(graph_state.alter(|_| None, key)).into();
                Ok(())
            }
            &[graph_id, ..] => {
                let id = graph_id.try_into().expect("node_id out of range");
                let key = SteelVal::IntV(id);
                let remove = |opt: Option<SteelVal>| match opt {
                    Some(SteelVal::HashMapV(mut nested)) => {
                        remove_hashmap_value(&mut nested, &node_path[1..])
                            .expect("failed to remove value");
                        Some(SteelVal::HashMapV(nested))
                    }
                    // No nested state found, nothing to remove.
                    other => other,
                };
                *graph_state = Gc::new(graph_state.alter(remove, key)).into();
                Ok(())
            }
        }
    }

    remove_hashmap_value(&mut root_state, node_path)?;
    vm.update_value(ROOT_STATE, SteelVal::HashMapV(root_state));
    Ok(())
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

/// Check if any value exists at the given path.
pub fn value_exists(vm: &Engine, path: &[node::Id]) -> Result<bool, SteelErr> {
    extract_value(vm, path).map(|opt| opt.is_some())
}

/// Check if a value of type `S` exists at the given path.
///
/// Returns `false` if no value exists, or if the value cannot be converted to `S`.
pub fn exists<S: FromSteelVal>(vm: &Engine, path: &[node::Id]) -> Result<bool, SteelErr> {
    match extract_value(vm, path)? {
        None => Ok(false),
        Some(val) => Ok(S::from_steelval(&val).is_ok()),
    }
}

/// Initialize state with a raw `SteelVal` only if no state is currently present.
///
/// Ensures registration is idempotent - calling it multiple times won't reset existing state.
pub fn init_value_if_absent(
    vm: &mut Engine,
    path: &[node::Id],
    init: impl FnOnce() -> SteelVal,
) -> Result<(), SteelErr> {
    if !value_exists(vm, path)? {
        update_value(vm, path, init())?;
    }
    Ok(())
}

/// Initialize state only if no value of type `S` is currently present.
///
/// Useful for nodes that require a specific state type.
pub fn init_if_absent<S: NodeState>(
    vm: &mut Engine,
    path: &[node::Id],
    init: impl FnOnce() -> S,
) -> Result<(), SteelErr> {
    if !exists::<S>(vm, path)? {
        let val = init().into_steelval()?;
        update_value(vm, path, val)?;
    }
    Ok(())
}
