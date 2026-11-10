//! Assembly of the whole module from the lowered levels. See [`module`].

use super::{
    Config, Entrypoint, EntrypointId, EvalKind, EvalSource, Meta, MetaTree, ModuleError, NodeConf,
    NodeConns, RoseTree, SourceMap, analysis, codegen, emit, error, ir, lower, names,
};
use crate::{
    Edge,
    node::{self, Node},
    visit::{self, Visitor},
};
use petgraph::visit::{Data, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, Visitable};
use std::collections::{BTreeMap, BTreeSet};
use steel::parser::ast::ExprKind;

/// Group entrypoint sources by graph level, that is by parent path.
///
/// Returns a map from level path to the entrypoints with sources at that
/// level. A single entrypoint with cross-level sources appears at multiple
/// levels.
fn group_sources_by_level(
    entrypoints: &[Entrypoint],
) -> BTreeMap<Vec<node::Id>, Vec<(EntrypointId, Vec<&EvalSource>)>> {
    let mut map: BTreeMap<Vec<node::Id>, BTreeMap<EntrypointId, Vec<&EvalSource>>> =
        BTreeMap::new();
    for ep in entrypoints {
        let id = ep.id();
        for src in &ep.0 {
            let parent = src.path[..src.path.len() - 1].to_vec();
            map.entry(parent)
                .or_default()
                .entry(id.clone())
                .or_default()
                .push(src);
        }
    }
    map.into_iter()
        .map(|(level, ep_map)| (level, ep_map.into_iter().collect()))
        .collect()
}

/// Bitwise-OR of the branch patterns. All patterns have equal width. The
/// result is the set of outputs a branching push-through can produce across
/// all its branch outcomes.
fn or_patterns(patterns: &[node::Conns]) -> Result<node::Conns, error::NodeConnsError> {
    let n = patterns.first().map_or(0, node::Conns::len);
    let bits: Vec<bool> = (0..n)
        .map(|i| patterns.iter().any(|p| p.get(i).unwrap_or(false)))
        .collect();
    node::Conns::try_from_slice(&bits).map_err(|_| error::TooManyConns(n).into())
}

/// Given a root gantz graph, generate the full module with all the functions
/// needed to execute it. That is a function per node variant, a graph fn per
/// nested level variant, and a function per entrypoint's evaluation.
///
/// Lowers through the join-point IR pipeline with the toggles in [`Config`]
/// applied. See the module docs for how nested graphs and the entrypoints
/// sourced inside them compile.
pub fn module<'a, G>(
    get_node: node::GetNode<'a>,
    g: G,
    entrypoints: &[Entrypoint],
    config: &Config,
) -> Result<Vec<ExprKind>, ModuleError>
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    let mut meta_tree = MetaTree::default();
    crate::graph::visit(get_node, g, &[], &mut meta_tree);
    if !meta_tree.errors.is_empty() {
        return Err(error::MetaErrors(meta_tree.errors).into());
    }
    let meta_tree = meta_tree.tree;

    let level_sources = group_sources_by_level(entrypoints);

    let mut builder = ModuleBuilder {
        meta_tree: &meta_tree,
        level_sources: &level_sources,
        config,
        confs: BTreeMap::new(),
        graph_fns: Vec::new(),
        fns: Vec::new(),
        lowered: BTreeSet::new(),
        node_fns: Vec::new(),
        state: BTreeMap::new(),
        node_errors: Vec::new(),
        lower_error: None,
    };

    // One entry fn per entrypoint. The recursive level walk lowers any nested
    // level fns and yields the root statements inline.
    let ep_ids: BTreeSet<EntrypointId> = entrypoints.iter().map(|ep| ep.id()).collect();
    for ep_id in &ep_ids {
        let open = ["define", crate::GRAPH_STATE, crate::ROOT_STATE];
        let close = ["set!", crate::ROOT_STATE, crate::GRAPH_STATE];
        let mut head = vec![emit::l(open.map(emit::a))];
        let mut body = ir::Body {
            steps: vec![],
            tail: ir::Tail::Ret(vec![]),
        };
        if let Some((glue, out)) = builder.ep_level_stmts(ep_id, &meta_tree, &[])? {
            head.extend(glue);
            body = out.body;
        }
        builder.fns.push(LoweredFn {
            name: names::entry_fn_name(ep_id),
            params: vec![],
            path: vec![],
            head,
            body,
            tail: emit::l(close.map(emit::a)),
        });
    }

    // With `emit_all_node_fns`, every node contributes its all-connected
    // variant. Its node fn is then emitted even when nothing reachable from
    // an entrypoint calls it.
    if config.emit_all_node_fns {
        all_connected_confs(&meta_tree, &mut Vec::new(), &mut builder.confs)?;
    }

    // A node fn per called variant, and a graph fn per variant that a node
    // fn calls.
    crate::graph::visit(get_node, g, &[], &mut builder);
    if let Some(error) = builder.lower_error {
        return Err(error);
    }
    if !builder.node_errors.is_empty() {
        return Err(error::NodeFnErrors(builder.node_errors).into());
    }

    // The lowered bodies emit last. A call site follows the state use of the
    // node fn variant it calls.
    let state = builder.state;
    let emit_fn = |f: LoweredFn| {
        let no_state = BTreeMap::new();
        let state = state.get(&f.path).unwrap_or(&no_state);
        let cx = emit::Cx {
            path: &f.path,
            state,
        };
        let mut stmts = f.head;
        stmts.extend(emit::steps_sexps(&cx, &f.body));
        stmts.push(f.tail);
        emit::fn_def(&f.name, &f.params, stmts)
    };

    // Steel resolves free identifiers at definition, so definition order
    // matters. Deepest levels come first. Within a depth, a level's interior
    // node fns precede the graph fns of the levels at that depth. A wrapper
    // node fn at depth D may call a graph fn at depth D+1. A graph fn calls
    // its interior node fns and deeper graph fns. Level and entry fns come
    // last.
    let mut fns: Vec<(usize, usize, ExprKind)> = builder
        .node_fns
        .into_iter()
        .map(|(depth, f)| (depth, 0, f))
        .collect();
    let graph_fns = builder.graph_fns.into_iter();
    fns.extend(graph_fns.map(|(depth, f)| (depth, 1, emit_fn(f))));
    fns.sort_by(|(d1, k1, _), (d2, k2, _)| d2.cmp(d1).then(k1.cmp(k2)));

    // One `(require ...)` per Steel module named by any node in the meta
    // tree, ahead of every definition. Steel resolves the free identifiers
    // of every emitted fn at definition time, so module bindings must be
    // required even for nodes outside any eval path. The `BTreeSet` union
    // keeps emission order deterministic.
    let requires = collect_requires(&meta_tree)
        .into_iter()
        .map(|name| {
            // Node-declared names reach a string literal in emitted Steel, so
            // restrict them to a conservative charset rather than escaping.
            let valid = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "/-_.".contains(c));
            if !valid {
                return Err(ModuleError::InvalidModuleName { name });
            }
            Ok(emit::parse_one(&format!("(require \"{name}\")")))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(requires
        .into_iter()
        .chain(fns.into_iter().map(|(_, _, f)| f))
        .chain(builder.fns.into_iter().map(emit_fn))
        .collect())
}

/// The union of [`Meta::requires`] over every level of the meta tree.
fn collect_requires(tree: &RoseTree<Meta>) -> BTreeSet<String> {
    let mut set = tree.elem.requires.clone();
    for sub in tree.nested.values() {
        set.extend(collect_requires(sub));
    }
    set
}

/// Insert the all-connected variant conf of every node at every level of the
/// meta tree. This is the [`Config::emit_all_node_fns`] set. Inlets, outlets
/// and delays have no node fns, so they are skipped.
fn all_connected_confs(
    tree: &RoseTree<Meta>,
    path: &mut Vec<node::Id>,
    confs: &mut BTreeMap<Vec<node::Id>, BTreeSet<NodeConf>>,
) -> Result<(), ModuleError> {
    let conn = |n: usize, path: &[node::Id]| {
        node::Conns::connected(n).map_err(|_| ModuleError::NodeConns {
            path: path.to_vec(),
            error: error::TooManyConns(n).into(),
        })
    };
    let meta = &tree.elem;
    let level_confs = confs.entry(path.clone()).or_default();
    for n in meta.graph.nodes() {
        if meta.inlets.contains(&n) || meta.outlets.contains(&n) || meta.delays.contains(&n) {
            continue;
        }
        let inputs = conn(meta.inputs.get(&n).copied().unwrap_or(0), path)?;
        let outputs = conn(meta.outputs.get(&n).copied().unwrap_or(0), path)?;
        let conns = NodeConns { inputs, outputs };
        level_confs.insert(NodeConf { id: n, conns });
    }
    for (&id, sub) in &tree.nested {
        path.push(id);
        all_connected_confs(sub, path, confs)?;
        path.pop();
    }
    Ok(())
}

/// A nested level's contribution to its parent's evaluation of one
/// entrypoint.
struct LevelPiece {
    /// The emitted level fn's name.
    fn_name: String,
    /// Whether the level's interior holds any state. If so, the fn threads
    /// the level's state hashmap in and out.
    stateful: bool,
    /// `Some` when the level's evaluation reaches its outlets. It holds the
    /// distinct activation patterns. With two or more, the parent dispatches
    /// per pattern.
    patterns: Option<Vec<node::Conns>>,
}

/// A lowered fn that waits for emission. A call site in its body follows the
/// state use of the node fn variant it calls, known once every node fn is
/// generated.
struct LoweredFn {
    name: String,
    params: Vec<String>,
    /// The path of the level that `body` was lowered from.
    path: Vec<node::Id>,
    /// The statements ahead of the body.
    head: Vec<emit::Sexp>,
    body: ir::Body,
    /// The statement after the body.
    tail: emit::Sexp,
}

/// The working state of one [`module`] invocation. It accumulates node
/// variant confs, lowered fns and node fns. The per-entrypoint level
/// recursion and the per-variant graph fn generation are methods. The node
/// fn generation is its [`Visitor`] impl.
struct ModuleBuilder<'a> {
    meta_tree: &'a RoseTree<Meta>,
    level_sources: &'a BTreeMap<Vec<node::Id>, Vec<(EntrypointId, Vec<&'a EvalSource>)>>,
    config: &'a Config,
    /// Node variants called by the lowered bodies, per level path.
    confs: BTreeMap<Vec<node::Id>, BTreeSet<NodeConf>>,
    /// The graph fn variants lowered so far, by level path and inlet mask.
    lowered: BTreeSet<(Vec<node::Id>, node::Conns)>,
    /// Lowered graph fns with their level depth. A graph fn calls only
    /// deeper graph fns, so they are defined deepest-first, before all level
    /// and entry fns. Steel resolves free identifiers at definition.
    graph_fns: Vec<(usize, LoweredFn)>,
    /// Lowered level and entry fns, child levels before their parents.
    fns: Vec<LoweredFn>,
    /// Generated node fns with the depth of the level of their node.
    node_fns: Vec<(usize, ExprKind)>,
    /// The state use of each stateful node fn variant, by level path.
    state: codegen::StateUses,
    node_errors: Vec<error::NodeExprError>,
    /// The first error from lowering a graph fn that a node fn calls.
    lower_error: Option<ModuleError>,
}

/// Generates the node fns of every called variant. A node fn names the graph
/// fn variants it calls. Each lowers when first named, and that adds the node
/// variants its body calls. A node is visited before the nodes of its nested
/// graph, so their variants are complete when the visit reaches them.
impl Visitor for ModuleBuilder<'_> {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &dyn Node) {
        // Inlet and outlet nodes get no node fn. Their values resolve as
        // bindings.
        let meta_ctx = node::MetaCtx::new(ctx.get_node());
        if matches!(node.role(meta_ctx), node::Role::Inlet | node::Role::Outlet) {
            return;
        }
        let node_path = ctx.path();
        let level = &node_path[..node_path.len() - 1];
        let id = ctx.id();
        let confs: Vec<NodeConf> = match self.confs.get(level) {
            Some(confs) => node_confs(confs, id).copied().collect(),
            None => return,
        };
        for conf in confs {
            let connected = codegen::connected_inputs(conf.conns.inputs, ctx.inputs());
            let node_fn =
                codegen::node_fn(ctx.get_node(), node, node_path, &conf.conns, &connected);
            let (fn_expr, state) = match node_fn {
                Ok(node_fn) => node_fn,
                Err(error) => {
                    let path = node_path.to_vec();
                    let error = error::NodeExprError { path, error };
                    self.node_errors.push(error);
                    continue;
                }
            };
            // Only a node with a nested graph has graph fns to name.
            let called = match self.meta_tree.tree(node_path) {
                Some(_) => SourceMap::parse(&fn_expr.to_string()),
                None => SourceMap::default(),
            };
            for occ in called.occs() {
                let names::Name::GraphFn { path, inputs } = &occ.name else {
                    continue;
                };
                if path == node_path && self.lowered.insert((path.clone(), *inputs)) {
                    let lowered = self.graph_fn(path, inputs);
                    self.lower_error = self.lower_error.take().or(lowered.err());
                }
            }
            self.node_fns.push((level.len(), fn_expr));
            if let Some(state) = state {
                self.state
                    .entry(level.to_vec())
                    .or_default()
                    .insert(conf, state);
            }
        }
    }
}

impl ModuleBuilder<'_> {
    /// Lower the evaluation of `ep` at the level `path`. Yields the glue
    /// statements that call child level fns, and this level's own lowered
    /// body. The glue threads state and binds results. Returns `None` when
    /// neither this level nor any descendant has sources for `ep`.
    fn ep_level_stmts(
        &mut self,
        ep: &EntrypointId,
        meta_node: &RoseTree<Meta>,
        path: &[node::Id],
    ) -> Result<Option<(Vec<emit::Sexp>, lower::LevelOut)>, ModuleError> {
        use emit::{Sexp, a, l};

        let meta = &meta_node.elem;
        let mut glue: Vec<Sexp> = Vec::new();
        let mut push: Vec<(node::Id, node::Conns)> = Vec::new();
        let mut pull: Vec<(node::Id, node::Conns)> = Vec::new();
        let mut extra_branches = BTreeMap::new();
        let mut prebound = BTreeSet::new();
        let mut prebound_vars: Vec<ir::Var> = Vec::new();
        let mut any_child = false;

        // Children first, in post-order. Each contributing child evaluates
        // via its level fn. One that reaches its outlets becomes a pre-bound
        // source this level continues from.
        for (&gid, sub_meta) in &meta_node.nested {
            let mut child_path = path.to_vec();
            child_path.push(gid);
            let Some(piece) = self.ep_level(ep, sub_meta, &child_path)? else {
                continue;
            };
            any_child = true;
            let pair = a(names::pair_name(gid));
            if piece.stateful {
                let s = format!("%lvl-s-{gid}");
                let r = format!("%lvl-r-{gid}");
                let call = l([a(piece.fn_name), a(s.clone())]);
                let new_state = l([a("list-ref"), a(r.clone()), a("1")]);
                glue.push(l([a("define"), a(s.clone()), emit::state_ref(gid)]));
                glue.push(l([a("define"), a(r.clone()), call]));
                glue.push(emit::state_update(gid, a(s), new_state));
                if piece.patterns.is_some() {
                    glue.push(l([
                        a("define"),
                        pair.clone(),
                        l([a("list-ref"), a(r), a("0")]),
                    ]));
                }
            } else if piece.patterns.is_some() {
                glue.push(l([a("define"), pair.clone(), l([a(piece.fn_name)])]));
            } else {
                // Stateless and no outlets reached. Evaluate for side effects.
                glue.push(l([a(piece.fn_name)]));
            }

            let Some(patterns) = piece.patterns else {
                continue;
            };
            let n_out = meta.outputs.get(&gid).copied().unwrap_or(0);
            prebound.insert(gid);
            if patterns.len() >= 2 {
                prebound_vars.push(ir::Var::Result { node: gid });
                let pattern_union =
                    or_patterns(&patterns).map_err(|error| ModuleError::NodeConns {
                        path: path.to_vec(),
                        error,
                    })?;
                push.push((gid, pattern_union));
                extra_branches.insert(gid, patterns);
            } else {
                // Destructure all outputs from the plain result.
                let out_name = |o: usize| {
                    names::var_name(&ir::Var::Output {
                        node: gid,
                        output: o,
                    })
                };
                match n_out {
                    0 => {}
                    1 => glue.push(l([a("define"), a(out_name(0)), pair])),
                    _ => {
                        for o in 0..n_out {
                            glue.push(l([
                                a("define"),
                                a(out_name(o)),
                                l([a("list-ref"), pair.clone(), a(o.to_string())]),
                            ]));
                        }
                    }
                }
                for o in 0..n_out {
                    prebound_vars.push(ir::Var::Output {
                        node: gid,
                        output: o,
                    });
                }
                let conns = node::Conns::connected(n_out).map_err(|_| ModuleError::NodeConns {
                    path: path.to_vec(),
                    error: error::TooManyConns(n_out).into(),
                })?;
                push.push((gid, conns));
            }
        }

        // This level's direct sources.
        for (id, srcs) in self.level_sources.get(path).map(|v| &v[..]).unwrap_or(&[]) {
            if id != ep {
                continue;
            }
            for src in srcs {
                let node_id = *src.path.last().unwrap();
                match src.kind {
                    EvalKind::Push => push.push((node_id, src.conns)),
                    EvalKind::Pull => pull.push((node_id, src.conns)),
                }
            }
        }

        if !any_child && push.is_empty() && pull.is_empty() {
            return Ok(None);
        }

        let cx = lower::Cx {
            meta,
            extra_branches,
            prebound,
        };
        let out =
            lower::level_body(&cx, &lower::LevelSources::Eval { push, pull }).map_err(|error| {
                ModuleError::Lower {
                    path: path.to_vec(),
                    error,
                }
            })?;
        if self.config.validate_ir {
            ir::validate(&out.body, 0, &prebound_vars).map_err(|e| ModuleError::InvalidIr {
                path: path.to_vec(),
                detail: e.to_string(),
            })?;
        }
        lower::collect_confs(&out.body, self.confs.entry(path.to_vec()).or_default());
        Ok(Some((glue, out)))
    }

    /// Build and emit the level fn evaluating `ep` at the nested level
    /// `path`, returning how the parent integrates it.
    fn ep_level(
        &mut self,
        ep: &EntrypointId,
        meta_node: &RoseTree<Meta>,
        path: &[node::Id],
    ) -> Result<Option<LevelPiece>, ModuleError> {
        let Some((glue, out)) = self.ep_level_stmts(ep, meta_node, path)? else {
            return Ok(None);
        };

        // Whether and how this level's evaluation produces its outlets.
        // Analysed over the lowered body itself.
        let produced = out.outlets.iter().any(|o| o.atom.is_some());
        let patterns =
            match produced {
                true => Some(analysis::outlet_patterns(&out.body, &out.outlets).map_err(
                    |error| ModuleError::NodeConns {
                        path: path.to_vec(),
                        error: error.into(),
                    },
                )?),
                false => None,
            };
        let result = match &patterns {
            Some(patterns) => emit::level_result_sexp(&out.outlets, patterns),
            None => emit::unit(),
        };

        let stateful = !meta_node.elem.stateful.is_empty();
        let fn_name = names::lvl_fn_name(ep, path);
        let (params, tail) = state_frame(stateful, vec![], result);
        self.fns.push(LoweredFn {
            name: fn_name.clone(),
            params,
            path: path.to_vec(),
            head: glue,
            body: out.body,
            tail,
        });
        Ok(Some(LevelPiece {
            fn_name,
            stateful,
            patterns,
        }))
    }

    /// Build and emit the graph fn for the nested level at `gpath`, for the
    /// variant invoked with the given active-input mask.
    fn graph_fn(&mut self, gpath: &[node::Id], imask: &node::Conns) -> Result<(), ModuleError> {
        let sub = self.meta_tree.tree(gpath).expect("nested level exists");
        let meta = &sub.elem;
        let inlet_ids: Vec<node::Id> = meta.inlets.iter().copied().collect();
        let active = active_inlets_from_conns(&inlet_ids, imask);
        let cx = lower::Cx {
            meta,
            extra_branches: BTreeMap::new(),
            prebound: BTreeSet::new(),
        };
        let out = lower::level_body(&cx, &lower::LevelSources::Inlets(active.clone())).map_err(
            |error| ModuleError::Lower {
                path: gpath.to_vec(),
                error,
            },
        )?;
        if self.config.validate_ir {
            let inlet_vars: Vec<ir::Var> = active
                .iter()
                .map(|&i| ir::Var::Output { node: i, output: 0 })
                .collect();
            ir::validate(&out.body, 0, &inlet_vars).map_err(|e| ModuleError::InvalidIr {
                path: gpath.to_vec(),
                detail: e.to_string(),
            })?;
        }
        lower::collect_confs(&out.body, self.confs.entry(gpath.to_vec()).or_default());

        // The node-contract patterns. The all-active analysis matches the
        // order `Graph::branches` reports to the parent.
        let patterns =
            analysis::level_branch_patterns(meta).map_err(|error| ModuleError::Lower {
                path: gpath.to_vec(),
                error,
            })?;
        let result = emit::level_result_sexp(&out.outlets, &patterns);
        let inlets: Vec<String> = active
            .iter()
            .map(|&i| names::var_name(&ir::Var::Output { node: i, output: 0 }))
            .collect();
        let (params, tail) = state_frame(!meta.stateful.is_empty(), inlets, result);
        let lowered = LoweredFn {
            name: names::graph_fn_name(gpath, imask),
            params,
            path: gpath.to_vec(),
            head: vec![],
            body: out.body,
            tail,
        };
        self.graph_fns.push((gpath.len(), lowered));
        Ok(())
    }
}

/// The parameters and the closing statement of a level fn or graph fn that
/// yields `result`. A fn of a level with state takes the state map of the
/// level as its last parameter and returns `(list result state)`.
fn state_frame(
    stateful: bool,
    mut params: Vec<String>,
    result: emit::Sexp,
) -> (Vec<String>, emit::Sexp) {
    use emit::{a, l};
    if !stateful {
        return (params, result);
    }
    params.push(crate::GRAPH_STATE.to_string());
    (params, l([a("list"), result, a(crate::GRAPH_STATE)]))
}

/// The inlet ids active for a parent input-conns mask. Bit `i` maps to
/// `inlet_ids[i]`. This is the input-to-inlet contract shared with
/// `node::graph::graph_call_expr`.
fn active_inlets_from_conns(inlet_ids: &[node::Id], conns: &node::Conns) -> BTreeSet<node::Id> {
    inlet_ids
        .iter()
        .enumerate()
        .filter(|(i, _)| conns.get(*i).unwrap_or(false))
        .map(|(_, &id)| id)
        .collect()
}

/// The variants of node `id` among the variants of its level.
fn node_confs(confs: &BTreeSet<NodeConf>, id: node::Id) -> impl Iterator<Item = &NodeConf> {
    let first = |id| NodeConf {
        id,
        conns: NodeConns {
            inputs: node::Conns::empty(),
            outputs: node::Conns::empty(),
        },
    };
    confs
        .range(first(id)..)
        .take_while(move |conf| conf.id == id)
}
