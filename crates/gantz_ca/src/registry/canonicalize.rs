//! Re-addressing a registry in canonical form. See
//! [`Registry::canonicalize`].

use super::Registry;
use crate::{
    CommitAddr, ContentAddr, Datum, GraphAddr, Head, Key, NodeData, Value, commit_addr, graph_addr,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::Hash;

/// The graphs and commits that [`Registry::canonicalize`] gave new
/// addresses, from old to new.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Moved {
    pub graphs: BTreeMap<GraphAddr, GraphAddr>,
    pub commits: BTreeMap<CommitAddr, CommitAddr>,
}

impl Moved {
    /// Whether nothing moved.
    pub fn is_empty(&self) -> bool {
        self.graphs.is_empty() && self.commits.is_empty()
    }

    /// The new address of graph `ga`.
    pub fn graph(&self, ga: GraphAddr) -> GraphAddr {
        self.graphs.get(&ga).copied().unwrap_or(ga)
    }

    /// The new address of commit `ca`.
    pub fn commit(&self, ca: CommitAddr) -> CommitAddr {
        self.commits.get(&ca).copied().unwrap_or(ca)
    }

    /// `head` after the move. A name stays as it is.
    pub fn head(&self, head: &Head) -> Head {
        match head {
            Head::Commit(ca) => Head::Commit(self.commit(*ca)),
            Head::Branch(_) => head.clone(),
        }
    }

    /// The new address of `addr`, as a graph or a commit.
    fn addr(&self, addr: ContentAddr) -> ContentAddr {
        let graph = self.graph(GraphAddr::from(addr));
        match ContentAddr::from(graph) == addr {
            true => self.commit(CommitAddr::from(addr)).into(),
            false => graph.into(),
        }
    }
}

impl Registry {
    /// Bring every node to canonical form, and give each graph and commit
    /// whose address changes its new address. Returns what moved.
    ///
    /// A move follows everything that points at the old address: the nodes
    /// that refer to a graph, through their `refs` and through address
    /// strings in their data, the commits of a graph and their children, and
    /// section keys and values. A store written before a change to the
    /// canonical form catches up this way. A canonical registry is left as
    /// it is.
    pub fn canonicalize(&mut self) -> Moved {
        let mut moved = Moved::default();
        // Each moved address as it appears in datum strings, old to new.
        let mut strs = HashMap::new();

        let mut graphs = std::mem::take(&mut self.graphs);
        let order = dependency_order(graphs.keys().copied(), |ga| {
            let refs = graphs[&ga].node_weights().flat_map(|n| n.refs.iter());
            refs.map(|r| GraphAddr::from(*r))
                .filter(|r| graphs.contains_key(r))
                .collect()
        });
        for ga in order {
            let Some(mut graph) = graphs.remove(&ga) else {
                continue;
            };
            for node in graph.node_weights_mut() {
                follow_moves(node, &moved, &strs);
            }
            let new = graph_addr(&graph);
            if new != ga {
                moved.graphs.insert(ga, new);
                strs.insert(ga.to_string(), new.to_string());
            }
            self.graphs.entry(new).or_insert(graph);
        }

        let mut commits = std::mem::take(&mut self.commits);
        let order = dependency_order(commits.keys().copied(), |ca| {
            let commit = &commits[&ca];
            let parents = commit.parent.iter().chain(&commit.merge_parents);
            parents
                .copied()
                .filter(|p| commits.contains_key(p))
                .collect()
        });
        for ca in order {
            let Some(mut commit) = commits.remove(&ca) else {
                continue;
            };
            commit.graph = moved.graph(commit.graph);
            commit.parent = commit.parent.map(|p| moved.commit(p));
            for p in &mut commit.merge_parents {
                *p = moved.commit(*p);
            }
            let new = commit_addr(&commit);
            if new != ca {
                moved.commits.insert(ca, new);
                strs.insert(ca.to_string(), new.to_string());
            }
            self.commits.entry(new).or_insert(commit);
        }

        for section in self.sections.values_mut() {
            let entries = std::mem::take(&mut section.entries);
            section.entries = entries
                .into_iter()
                .map(|(key, mut value)| {
                    let key = match key {
                        Key::Name(name) => Key::Name(name),
                        Key::Commit(ca) => Key::Commit(moved.commit(ca)),
                        Key::Graph(ga) => Key::Graph(moved.graph(ga)),
                        Key::Addr(addr) => Key::Addr(moved.addr(addr)),
                    };
                    match &mut value {
                        Value::Commit(ca) => *ca = moved.commit(*ca),
                        Value::Datum(datum) => {
                            rename_strs(datum, &strs);
                            datum.canonicalize();
                        }
                        Value::Blob(..) => (),
                    }
                    (key, value)
                })
                .collect();
        }
        moved
    }
}

/// Point `node` at the moved graphs, then canonicalize it.
fn follow_moves(node: &mut NodeData, moved: &Moved, strs: &HashMap<String, String>) {
    for r in &mut node.refs {
        *r = moved.graph(GraphAddr::from(*r)).into();
    }
    rename_strs(&mut node.data, strs);
    node.canonicalize();
}

/// Replace each string in `datum` that `strs` renames.
fn rename_strs(datum: &mut Datum, strs: &HashMap<String, String>) {
    match datum {
        Datum::Str(s) => {
            if let Some(new) = strs.get(s.as_str()) {
                s.clone_from(new);
            }
        }
        Datum::Seq(items) => items.iter_mut().for_each(|d| rename_strs(d, strs)),
        Datum::Map(entries) => entries.iter_mut().for_each(|(_, d)| rename_strs(d, strs)),
        _ => (),
    }
}

/// Every key in `keys`, each after the keys it depends on. Walks with an
/// explicit stack, since a history can be far deeper than the call stack.
fn dependency_order<K: Copy + Eq + Hash + Ord>(
    keys: impl Iterator<Item = K>,
    deps: impl Fn(K) -> Vec<K>,
) -> Vec<K> {
    let mut roots: Vec<K> = keys.collect();
    roots.sort();
    let mut order = Vec::with_capacity(roots.len());
    let mut entered = HashSet::new();
    for root in roots {
        let mut stack = vec![(root, false)];
        while let Some((key, deps_done)) = stack.pop() {
            if deps_done {
                order.push(key);
            } else if entered.insert(key) {
                stack.push((key, true));
                let pending = deps(key).into_iter().filter(|d| !entered.contains(d));
                stack.extend(pending.map(|d| (d, false)));
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Commit, DataGraph, Liveness, MergePolicy, Name, sync::verify_graph};
    use std::collections::BTreeSet;
    use std::time::Duration;

    fn node(data: Vec<(&str, Datum)>) -> NodeData {
        let data = data.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        NodeData::new("test", Datum::Map(data))
    }

    fn graph(nodes: Vec<NodeData>) -> DataGraph {
        let mut graph = DataGraph::default();
        for n in nodes {
            graph.add_node(n);
        }
        graph
    }

    /// Insert `graph` under the address of its form as given, canonical or
    /// not, as an older store holds it.
    fn stored_graph(reg: &mut Registry, graph: DataGraph) -> GraphAddr {
        let ga = graph_addr(&graph);
        reg.insert_graph_at(ga, graph);
        ga
    }

    fn stored_commit(
        reg: &mut Registry,
        secs: u64,
        parent: Option<CommitAddr>,
        ga: GraphAddr,
    ) -> CommitAddr {
        let commit = Commit::new(Duration::from_secs(secs), parent, ga);
        let ca = commit_addr(&commit);
        reg.insert_commit_at(ca, commit);
        ca
    }

    #[test]
    fn a_canonical_registry_stays_put() {
        let mut reg = Registry::default();
        let ga = reg.add_graph(graph(vec![node(vec![("n", Datum::I64(3))])]));
        let ca = reg.add_commit(Commit::new(Duration::from_secs(1), None, ga));
        reg.set_head("jam".parse().unwrap(), ca);
        let addrs = |reg: &Registry| {
            let graphs: BTreeSet<GraphAddr> = reg.graphs().keys().copied().collect();
            let commits: BTreeSet<CommitAddr> = reg.commits().keys().copied().collect();
            (graphs, commits)
        };
        let before = addrs(&reg);
        assert!(reg.canonicalize().is_empty());
        assert_eq!(addrs(&reg), before);
        assert_eq!(reg.head(&"jam".parse().unwrap()), Some(ca));
    }

    #[test]
    fn a_move_carries_referrers_commits_and_sections() {
        let mut reg = Registry::default();
        // A nested graph that holds an integer in its old form, and a graph
        // that refers to it by `refs` and by an address string.
        let inner = stored_graph(&mut reg, graph(vec![node(vec![("n", Datum::U64(3))])]));
        let mut referrer = node(vec![("graph", Datum::Str(inner.to_string()))]);
        referrer.refs.push(inner.into());
        let outer = stored_graph(&mut reg, graph(vec![referrer]));
        let still = reg.add_graph(graph(vec![node(vec![("n", Datum::I64(-1))])]));
        let root = stored_commit(&mut reg, 1, None, inner);
        let tip = stored_commit(&mut reg, 2, Some(root), outer);
        let jam: Name = "jam".parse().unwrap();
        reg.set_head(jam.clone(), tip);
        let view = Value::Datum(Datum::Str(tip.to_string()));
        let (policy, liveness) = (MergePolicy::KeepExisting, Liveness::WithCommit);
        reg.set_section_value("test.view", policy, liveness, Key::Commit(tip), view);

        let moved = reg.canonicalize();
        let (new_inner, new_outer) = (moved.graph(inner), moved.graph(outer));
        let (new_root, new_tip) = (moved.commit(root), moved.commit(tip));
        assert_ne!(new_inner, inner);
        assert_ne!(new_outer, outer);
        assert_ne!(new_tip, tip);
        assert_eq!(moved.graph(still), still);
        for (ga, graph) in reg.graphs() {
            verify_graph(*ga, graph).unwrap();
        }
        assert!(!reg.graphs().contains_key(&inner));
        let referrer = &reg.graphs()[&new_outer].node_weights().next().unwrap();
        assert_eq!(referrer.refs, vec![ContentAddr::from(new_inner)]);
        assert_eq!(
            referrer.data.get("graph"),
            Some(&Datum::Str(new_inner.to_string()))
        );
        let tip_commit = &reg.commits()[&new_tip];
        assert_eq!(
            (tip_commit.parent, tip_commit.graph),
            (Some(new_root), new_outer)
        );
        assert_eq!(reg.head(&jam), Some(new_tip));
        let view = reg.section_entry("test.view", &Key::Commit(new_tip));
        assert_eq!(view, Some(&Value::Datum(Datum::Str(new_tip.to_string()))));
        assert_eq!(moved.head(&Head::Commit(tip)), Head::Commit(new_tip));
        assert!(reg.canonicalize().is_empty());
    }

    #[test]
    fn a_long_history_moves_without_recursion() {
        let mut reg = Registry::default();
        let ga = stored_graph(&mut reg, graph(vec![node(vec![("n", Datum::U64(3))])]));
        let mut tip = None;
        for secs in 0..20_000 {
            tip = Some(stored_commit(&mut reg, secs, tip, ga));
        }
        let moved = reg.canonicalize();
        assert_eq!(moved.commits.len(), 20_000);
        let mut ca = moved.commit(tip.unwrap());
        while let Some(parent) = reg.commits()[&ca].parent {
            ca = parent;
        }
        assert_eq!(reg.commits()[&ca].timestamp, Duration::from_secs(0));
    }
}
