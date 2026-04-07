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
    steel_vm::{engine::Engine, register_fn::RegisterFn},
};

/// Opaque serialized byte payload with hex-aware serde.
///
/// In human-readable formats (RON, JSON) serializes as a hex string.
/// In binary formats serializes as raw bytes.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl Bytes {
    /// Consume the wrapper and return the inner `Vec<u8>`.
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

impl std::ops::Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl std::ops::DerefMut for Bytes {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

impl std::fmt::Debug for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const MAX_HEX: usize = 64;
        let hex = hex::encode(&self.0);
        if hex.len() <= MAX_HEX {
            write!(f, "Bytes({hex})")
        } else {
            write!(f, "Bytes({}...[{}])", &hex[..MAX_HEX], self.0.len())
        }
    }
}

impl std::fmt::Display for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&hex::encode(&self.0))
    }
}

impl Serialize for Bytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.serialize_str(&hex::encode(&self.0))
        } else {
            serializer.serialize_bytes(&self.0)
        }
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            hex::decode(&s).map(Bytes).map_err(serde::de::Error::custom)
        } else {
            struct BytesVisitor;
            impl<'de> serde::de::Visitor<'de> for BytesVisitor {
                type Value = Bytes;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("byte array")
                }
                fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Bytes, E> {
                    Ok(Bytes(v))
                }
                fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Bytes, E> {
                    Ok(Bytes(v.to_vec()))
                }
            }
            deserializer.deserialize_byte_buf(BytesVisitor)
        }
    }
}

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
    S: NodeState + 'static,
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

/// Extract the entire ROOT_STATE from the VM.
pub fn extract_root(vm: &Engine) -> Result<SteelVal, SteelErr> {
    vm.extract_value(ROOT_STATE)
}

/// Replace the entire ROOT_STATE in the VM.
pub fn restore_root(vm: &mut Engine, state: SteelVal) {
    vm.update_value(ROOT_STATE, state);
}

/// Serialize an arbitrary `SteelVal` to `Bytes` via Steel's `serialize-value`.
fn serialize_steelval(vm: &mut Engine, val: SteelVal) -> Result<Bytes, SteelErr> {
    use std::sync::{Arc, Mutex};

    let buf: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let buf_clone = buf.clone();
    vm.register_fn("__gantz-capture-bytes!", move |v: Vec<isize>| -> bool {
        let bytes: Vec<u8> = v.into_iter().map(|i| i as u8).collect();
        *buf_clone.lock().unwrap() = Some(bytes);
        true
    });

    vm.register_value("__gantz-serialize-tmp", val);
    vm.run(
        "(__gantz-capture-bytes! (bytes->list (serialized->bytes (serialize-value __gantz-serialize-tmp))))",
    )?;

    let bytes = buf
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| SteelErr::new(ErrorKind::Generic, "byte capture failed".into()))?;
    Ok(Bytes(bytes))
}

/// Deserialize `Bytes` back to a `SteelVal` via Steel's `deserialize-value`.
fn deserialize_steelval(vm: &mut Engine, bytes: &[u8]) -> Result<SteelVal, SteelErr> {
    use steel::rvals::SteelByteVector;
    let bv = SteelByteVector::new(bytes.to_vec());
    vm.register_value("__gantz-deserialize-tmp", SteelVal::ByteVector(bv));
    let results = vm.run("(deserialize-value (bytes->serialized __gantz-deserialize-tmp))")?;
    results.into_iter().last().ok_or_else(|| {
        SteelErr::new(
            ErrorKind::Generic,
            "deserialize-value returned no result".into(),
        )
    })
}

/// Serialize ROOT_STATE to bytes via Steel's `serialize-value` and `serialized->bytes`.
///
/// Returns an error if the state contains values that Steel cannot serialize
/// (e.g. futures, continuations, iterators).
pub fn serialize_root(vm: &mut Engine) -> Result<Bytes, SteelErr> {
    let root = extract_root(vm)?;
    serialize_steelval(vm, root)
}

/// Deserialize bytes and restore as ROOT_STATE.
///
/// The bytes must have been produced by [`serialize_root`].
pub fn deserialize_and_restore_root(vm: &mut Engine, bytes: &[u8]) -> Result<(), SteelErr> {
    let state = deserialize_steelval(vm, bytes)?;
    restore_root(vm, state);
    Ok(())
}

/// Serialize the state subtree at `path` (e.g. `[3]` or `[3, 1]`).
///
/// Returns `None` if no entry exists at that path.
pub fn serialize_entry(vm: &mut Engine, path: &[usize]) -> Result<Option<Bytes>, SteelErr> {
    let Some(val) = extract_value(vm, path)? else {
        return Ok(None);
    };
    serialize_steelval(vm, val).map(Some)
}

/// Deserialize bytes and insert at `path` in `%root-state`.
///
/// The bytes must have been produced by [`serialize_entry`] or `serialize_steelval`.
pub fn deserialize_and_restore_entry(
    vm: &mut Engine,
    path: &[usize],
    bytes: &[u8],
) -> Result<(), SteelErr> {
    let val = deserialize_steelval(vm, bytes)?;
    update_value(vm, path, val)
}
