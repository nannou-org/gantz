use super::{Deserialize, Serialize};
use crate::{
    ROOT_STATE,
    datum::{self, Datum},
    node::{self, Node},
};
use gantz_ca::{CommitAddr, Head, Liveness, MergePolicy, Name, Registry, SectionDecl, section_get};
use steel::{
    SteelErr, SteelVal,
    gc::Gc,
    rerrs::ErrorKind,
    rvals::{FromSteelVal, IntoSteelVal, SteelHashMap},
    steel_vm::engine::Engine,
};

/// A wrapper around a `Node` that adds some persistent state.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct State<N, S> {
    /// The node being wrapped with state.
    pub node: N,
    /// The type of state used by the node.
    pub state: core::marker::PhantomData<S>,
}

/// Types that may be used as state for a [`Node`].
// FIXME: Check whether `derive(Steel)` already covers this trait. If not, add
// a `derive` for it.
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

/// A trait implemented for all `Node` types allowing to add some state
/// accessible to its expression. This is particularly useful for adding state
/// to `Expr` nodes.
pub trait WithStateType: Node + Sized {
    /// Consume `self` and return a `Node` that has state of type `state_type`.
    fn with_state_type<S: NodeState>(self) -> State<Self, S> {
        State::<Self, S>::new(self)
    }
}

/// The state of the nodes of a graph as data: the path and the state of each
/// node, in path order.
///
/// A node state with no data form is left out, such as one that holds a
/// function or a runtime object. That node starts from its initial state on a
/// restore.
pub type Snapshot = Vec<(Vec<node::Id>, Datum)>;

/// The registry section of the node state of a commit, as a [`Snapshot`].
///
/// An entry rides with its commit, as a scene view does. So it syncs to a peer
/// once, when the commit does, and a state change makes no sync traffic.
pub struct Snapshots;

/// The registry section of the named graphs that do not keep their node state.
/// A head keeps its state unless its name is here.
///
/// An entry rides with the name, so the choice survives an edit.
pub struct Volatile;

/// The id of the [`Snapshots`] section.
pub const SNAPSHOTS_ID: &str = "gantz.state";

/// The id of the [`Volatile`] section.
pub const VOLATILE_ID: &str = "gantz.state.volatile";

/// The ident of the node identity tree. It holds an opaque identity value per
/// node, at the node's path followed by `ID_KEY`, with the same nesting as
/// [`ROOT_STATE`].
///
/// [`remove_value`], [`move_value`] and [`remap_root`] apply to identities as
/// they do to state. So a node keeps its identity for as long as it keeps its
/// state slot, stateful or not.
pub const ROOT_IDS: &str = "%root-ids";

/// The key under a node's path in [`ROOT_IDS`] that holds its identity. No
/// node index reaches it.
const ID_KEY: usize = isize::MAX as usize;

impl SectionDecl for Snapshots {
    const ID: &'static str = SNAPSHOTS_ID;
    const POLICY: MergePolicy = MergePolicy::KeepExisting;
    const LIVENESS: Liveness = Liveness::WithCommit;
    type Key = CommitAddr;
    type Value = Snapshot;
}

impl SectionDecl for Volatile {
    const ID: &'static str = VOLATILE_ID;
    const POLICY: MergePolicy = MergePolicy::KeepExisting;
    const LIVENESS: Liveness = Liveness::WithName;
    type Key = Name;
    type Value = bool;
}

impl<N, S> State<N, S> {
    /// Given some node, return a `State` node enabling access to state of the
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

    fn role(&self, ctx: node::MetaCtx) -> node::Role {
        self.node.role(ctx)
    }

    fn stateful(&self, _ctx: node::MetaCtx) -> bool {
        true
    }

    fn register(&self, ctx: node::RegCtx<'_, '_>) {
        let (get_node, path, vm) = ctx.into_parts();
        // Register the state type and its fns only once. Steel's
        // `register_value` and `register_fn` allocate a new global slot and
        // shadow the previous binding rather than overwriting it. The engine
        // persists across recompiles, so re-running this on every recompile
        // would leak. `register_type` binds the predicate under `S::NAME`, so
        // its presence means `S` is set up.
        if vm.extract_value(S::NAME).is_err() {
            S::register(vm);
        }
        if extract_value(vm, path).ok().flatten().is_none() {
            let val = default_node_state_steel_val::<S>();
            update(vm, path, val).unwrap();
        }
        self.node.register(node::RegCtx::new(get_node, path, vm));
    }

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        self.node.required_addrs()
    }

    fn required_blobs(&self) -> Vec<(gantz_ca::SectionId, gantz_ca::ContentAddr)> {
        self.node.required_blobs()
    }

    fn required_modules(&self, ctx: node::MetaCtx) -> Vec<String> {
        self.node.required_modules(ctx)
    }

    fn visit(&self, ctx: node::visit::Ctx<'_, '_>, visitor: &mut dyn node::Visitor) {
        self.node.visit(ctx, visitor)
    }
}

/// The state of the nodes in `vm` as data.
///
/// This reads the state tree alone, so it needs no graph or node lookup. A
/// map with only integer keys is the state of a nested graph.
pub fn snapshot(vm: &Engine) -> Snapshot {
    let mut nodes = vec![];
    if let Ok(root) = root_map(vm, ROOT_STATE) {
        collect_states(&root, &mut vec![], &mut nodes);
    }
    nodes.sort_by(|(a, _), (b, _)| a.cmp(b));
    nodes
}

/// Set each node in `nodes` that has state in `vm` to its value in `nodes`.
///
/// Call this after the graph registers. It writes only the slots of the nodes
/// that registered, so an entry that does not fit the graph changes nothing.
pub fn restore(vm: &mut Engine, nodes: &[(Vec<node::Id>, Datum)]) -> Result<(), SteelErr> {
    for (path, state) in nodes {
        match extract_value(vm, path) {
            Ok(Some(SteelVal::HashMapV(map))) if is_graph_state(&map) => (),
            Ok(Some(_)) => update_value(vm, path, datum::to_steel(state))?,
            Ok(None) | Err(_) => (),
        }
    }
    Ok(())
}

/// Set the nodes in `vm`, which registered the graph of `head`, to the state
/// stored for its commit, if `head` keeps its state.
pub fn restore_head(registry: &Registry, head: &Head, vm: &mut Engine) -> Result<(), SteelErr> {
    let commit = registry
        .head_commit_ca(head)
        .filter(|_| keeps(registry, head));
    match commit.and_then(|ca| section_get::<Snapshots>(registry, &ca)) {
        Some(nodes) => restore(vm, &nodes),
        None => Ok(()),
    }
}

/// Whether `head` keeps its node state. See [`Volatile`].
pub fn keeps(registry: &Registry, head: &Head) -> bool {
    match head {
        Head::Branch(name) => section_get::<Volatile>(registry, name).is_none(),
        Head::Commit(_) => true,
    }
}

/// `nodes` with the node at the root of each path changed by `ids`, a list of
/// `(from, to)` node indices. A node that `ids` does not hold drops out.
pub fn remap_snapshot(
    nodes: &[(Vec<node::Id>, Datum)],
    ids: impl IntoIterator<Item = (node::Id, node::Id)>,
) -> Snapshot {
    let ids: std::collections::BTreeMap<_, _> = ids.into_iter().collect();
    let remap = |(path, state): &(Vec<node::Id>, Datum)| {
        let (id, nested) = path.split_first()?;
        let root = std::iter::once(*ids.get(id)?);
        Some((root.chain(nested.iter().copied()).collect(), state.clone()))
    };
    nodes.iter().filter_map(remap).collect()
}

/// Sets the given node's state to the given value.
pub fn update_value(vm: &mut Engine, node_path: &[usize], val: SteelVal) -> Result<(), SteelErr> {
    update_in(vm, ROOT_STATE, node_path, val)
}

/// Sets the given node's state to the given value.
pub fn update<S: IntoSteelVal>(
    vm: &mut Engine,
    node_path: &[usize],
    val: S,
) -> Result<(), SteelErr> {
    update_value(vm, node_path, val.into_steelval()?)
}

/// Remove the state value at the given node path, with the node's identity.
///
/// For a path like `[5]`, removes key `5` from `%root-state`.
/// For a path like `[5, 3]`, traverses into `%root-state[5]` and removes key `3`.
///
/// No-op if the key does not exist or if `ROOT_STATE` is not initialized.
pub fn remove_value(vm: &mut Engine, node_path: &[usize]) -> Result<(), SteelErr> {
    remove_in(vm, ROOT_STATE, node_path)?;
    remove_in(vm, ROOT_IDS, node_path)
}

/// Move the state value at `from` to `to`, clearing `from`. The node's
/// identity moves with it.
///
/// The moved value carries any nested subtree with it, so this rekeys a whole
/// node's state in one step. A no-op if there is no value at `from`. Used to
/// migrate the swapped node after a plain `petgraph::Graph` swap-remove keeps
/// node indices contiguous. See [`crate::node::graph::Graph`].
pub fn move_value(vm: &mut Engine, from: &[usize], to: &[usize]) -> Result<(), SteelErr> {
    move_in(vm, ROOT_STATE, from, to)?;
    if vm.extract_value(ROOT_IDS).is_ok() {
        move_in(vm, ROOT_IDS, from, to)?;
    }
    Ok(())
}

/// Re-key the root graph's per-node state and identities through `mapping`
/// in a single pass. The mapping is from old node index to new node index.
///
/// Each moved value carries its nested subtree with it. State for indices
/// absent from the mapping is dropped. Unlike repeated [`move_value`] calls,
/// an arbitrary permutation cannot collide with itself here. Used when a whole
/// graph is rebuilt with a known node correspondence, for example merging a
/// diverged branch. A no-op if `ROOT_STATE` is not initialized.
pub fn remap_root(
    vm: &mut Engine,
    mapping: &std::collections::BTreeMap<usize, usize>,
) -> Result<(), SteelErr> {
    remap_in(vm, ROOT_STATE, mapping)?;
    remap_in(vm, ROOT_IDS, mapping)
}

/// Extract the value for the node with the given ID.
pub fn extract_value(vm: &Engine, node_path: &[usize]) -> Result<Option<SteelVal>, SteelErr> {
    extract_in(vm, ROOT_STATE, node_path)
}

/// The identity of the node at `node_path`, if it has one. See [`ROOT_IDS`].
pub fn id(vm: &Engine, node_path: &[usize]) -> Result<Option<SteelVal>, SteelErr> {
    if vm.extract_value(ROOT_IDS).is_err() {
        return Ok(None);
    }
    extract_in(vm, ROOT_IDS, &id_path(node_path))
}

/// Set the identity of the node at `node_path`. See [`ROOT_IDS`].
pub fn set_id(vm: &mut Engine, node_path: &[usize], id: SteelVal) -> Result<(), SteelErr> {
    if vm.extract_value(ROOT_IDS).is_err() {
        vm.register_value(ROOT_IDS, SteelVal::empty_hashmap());
    }
    update_in(vm, ROOT_IDS, &id_path(node_path), id)
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
/// Ensures registration is idempotent. Calling it multiple times does not
/// reset existing state.
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

/// Add the state of each node in `map`, the state of the graph at `path`, to
/// `nodes`.
fn collect_states(map: &SteelHashMap, path: &mut Vec<node::Id>, nodes: &mut Snapshot) {
    for (key, val) in map.iter() {
        let SteelVal::IntV(id) = key else {
            continue;
        };
        let Ok(id) = usize::try_from(*id) else {
            continue;
        };
        path.push(id);
        match val {
            SteelVal::HashMapV(nested) if is_graph_state(nested) => {
                collect_states(nested, path, nodes)
            }
            val => match datum::of_steel(val) {
                Ok(state) => nodes.push((path.clone(), state)),
                Err(e) => log::debug!("no snapshot of the state at {path:?}: {e}"),
            },
        }
        path.pop();
    }
}

/// Whether `map` is the state of a graph, keyed by node index.
fn is_graph_state(map: &SteelHashMap) -> bool {
    !map.is_empty() && map.keys().all(|key| matches!(key, SteelVal::IntV(_)))
}

/// The path in [`ROOT_IDS`] of the identity of the node at `node_path`.
fn id_path(node_path: &[usize]) -> Vec<usize> {
    node_path.iter().copied().chain([ID_KEY]).collect()
}

/// The state tree registered as `root`.
fn root_map(vm: &Engine, root: &str) -> Result<SteelHashMap, SteelErr> {
    match vm.extract_value(root)? {
        SteelVal::HashMapV(map) => Ok(map),
        _ => Err(SteelErr::new(
            ErrorKind::Generic,
            format!("`{root}` was not a hashmap"),
        )),
    }
}

/// Set the value at `node_path` in the state tree `root`.
fn update_in(
    vm: &mut Engine,
    root: &str,
    node_path: &[usize],
    val: SteelVal,
) -> Result<(), SteelErr> {
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

    let mut root_state = root_map(vm, root)?;
    update_hashmap_value(&mut root_state, node_path, val)?;
    vm.update_value(root, SteelVal::HashMapV(root_state));
    Ok(())
}

/// Remove the value at `node_path` in the state tree `root`. A no-op if
/// `root` is not initialized.
fn remove_in(vm: &mut Engine, root: &str, node_path: &[usize]) -> Result<(), SteelErr> {
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

    if vm.extract_value(root).is_err() {
        // No state initialized yet, nothing to remove.
        return Ok(());
    }
    let mut root_state = root_map(vm, root)?;
    remove_hashmap_value(&mut root_state, node_path)?;
    vm.update_value(root, SteelVal::HashMapV(root_state));
    Ok(())
}

/// Move the value at `from` to `to` in the state tree `root`.
fn move_in(vm: &mut Engine, root: &str, from: &[usize], to: &[usize]) -> Result<(), SteelErr> {
    if let Some(val) = extract_in(vm, root, from)? {
        update_in(vm, root, to, val)?;
        remove_in(vm, root, from)?;
    }
    Ok(())
}

/// Re-key the root level of the state tree `root` through `mapping`. A no-op
/// if `root` is not initialized.
fn remap_in(
    vm: &mut Engine,
    root: &str,
    mapping: &std::collections::BTreeMap<usize, usize>,
) -> Result<(), SteelErr> {
    if vm.extract_value(root).is_err() {
        // No state initialized yet, nothing to remap.
        return Ok(());
    }
    let root_state = root_map(vm, root)?;
    let mut remapped = steel::HashMap::new();
    for (key, val) in root_state.iter() {
        let &SteelVal::IntV(old) = key else {
            // Non-index keys are not expected at the root. Keep them as-is.
            remapped = remapped.update(key.clone(), val.clone());
            continue;
        };
        let old: usize = old.try_into().expect("node index out of range");
        if let Some(&new) = mapping.get(&old) {
            let new = new.try_into().expect("node index out of range");
            remapped = remapped.update(SteelVal::IntV(new), val.clone());
        }
    }
    vm.update_value(root, SteelVal::HashMapV(Gc::new(remapped).into()));
    Ok(())
}

/// The value at `node_path` in the state tree `root`.
fn extract_in(vm: &Engine, root: &str, node_path: &[usize]) -> Result<Option<SteelVal>, SteelErr> {
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

    let root_state = root_map(vm, root)?;
    extract_hashmap_value(&root_state, node_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A VM with empty root state and an identity at each path.
    fn vm(paths: &[&[usize]]) -> Engine {
        let mut vm = Engine::new_base();
        vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
        for (ix, path) in paths.iter().enumerate() {
            set_id(&mut vm, path, SteelVal::IntV(ix as isize)).unwrap();
        }
        vm
    }

    fn id_at(vm: &Engine, path: &[usize]) -> Option<SteelVal> {
        id(vm, path).unwrap()
    }

    /// A swap-remove moves the swapped node's identity with its nested
    /// identities, and drops the removed node's.
    #[test]
    fn identities_follow_a_swap_remove() {
        let mut vm = vm(&[&[0], &[1], &[2], &[2, 0]]);
        remove_value(&mut vm, &[1]).unwrap();
        move_value(&mut vm, &[2], &[1]).unwrap();
        assert_eq!(id_at(&vm, &[0]), Some(SteelVal::IntV(0)));
        assert_eq!(id_at(&vm, &[1]), Some(SteelVal::IntV(2)));
        assert_eq!(id_at(&vm, &[1, 0]), Some(SteelVal::IntV(3)));
        assert_eq!(id_at(&vm, &[2]), None);
    }

    /// A root remap re-keys identities and drops the unmapped ones.
    #[test]
    fn identities_follow_a_root_remap() {
        let mut vm = vm(&[&[0], &[1], &[1, 4]]);
        let mapping = [(1, 0)].into_iter().collect();
        remap_root(&mut vm, &mapping).unwrap();
        assert_eq!(id_at(&vm, &[0]), Some(SteelVal::IntV(1)));
        assert_eq!(id_at(&vm, &[0, 4]), Some(SteelVal::IntV(2)));
        assert_eq!(id_at(&vm, &[1]), None);
    }

    /// Identities do not show in the state tree, and state migrations work
    /// on a VM without identities.
    #[test]
    fn identities_are_apart_from_state() {
        let mut vm = vm(&[&[0]]);
        assert_eq!(extract_value(&vm, &[0]).unwrap(), None);
        let mut bare = Engine::new_base();
        bare.register_value(ROOT_STATE, SteelVal::empty_hashmap());
        update_value(&mut bare, &[3], SteelVal::IntV(7)).unwrap();
        move_value(&mut bare, &[3], &[0]).unwrap();
        remove_value(&mut vm, &[0]).unwrap();
        assert_eq!(extract_value(&bare, &[0]).unwrap(), Some(SteelVal::IntV(7)));
        assert_eq!(id_at(&bare, &[0]), None);
        assert_eq!(id_at(&vm, &[0]), None);
    }
}
