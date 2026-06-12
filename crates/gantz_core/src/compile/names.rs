//! The emitted Steel identifier naming scheme.
//!
//! Every identifier the compiler emits is produced by a formatter in this
//! module, and [`parse`] is its inverse: it maps an emitted identifier back
//! to the node, graph level, entrypoint or join it was generated for. Keeping
//! both directions here is what stops them drifting apart - if a formatter
//! changes, the roundtrip tests below fail.

use crate::{
    compile::{
        EntrypointId,
        ir::{JoinId, Var},
    },
    node,
};

/// A parsed emitted identifier.
///
/// `NodeFn`, `GraphFn` and `LvlFn` carry *full* paths (from the root graph),
/// as their formatters embed the whole path. `Output`, `Input`, `Result` and
/// `Join` are bindings local to one emitted definition: their ids are
/// relative to the graph level that definition was lowered from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Name {
    /// A per-variant node fn: `node-fn-{path}[-i{conns}][-o{conns}]`.
    NodeFn {
        path: Vec<node::Id>,
        inputs: node::Conns,
        outputs: node::Conns,
    },
    /// A per-variant graph fn: `graph-fn-{path}[-i{conns}]`.
    GraphFn {
        path: Vec<node::Id>,
        inputs: node::Conns,
    },
    /// A per-entrypoint nested level fn: `lvl-fn-{ep}-{path}`.
    LvlFn { ep: String, path: Vec<node::Id> },
    /// An entrypoint fn: `entry-fn-{ep}`.
    EntryFn { ep: String },
    /// A node output binding: `node-{id}-o{output}`.
    Output { node: node::Id, output: usize },
    /// A join param carrying a node input: `node-{id}-i{input}`.
    Input { node: node::Id, input: usize },
    /// A branching node's raw `(branch-ix value)` pair: `node-{id}`.
    Result { node: node::Id },
    /// A join point fn: `join-{id}`.
    Join { id: node::Id },
}

/// The string used to represent a path in a fn name.
pub(crate) fn path_string(path: &[node::Id]) -> String {
    path.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(":")
}

/// Generate a function name for a node based on its path in the graph.
///
/// E.g. `node-fn-0:1:2-i0101-o1100`.
pub(crate) fn node_fn_name(
    node_path: &[node::Id],
    inputs: &node::Conns,
    outputs: &node::Conns,
) -> String {
    let path_string = path_string(node_path);
    let inputs_prefix = if inputs.is_empty() { "" } else { "-i" };
    let outputs_prefix = if outputs.is_empty() { "" } else { "-o" };
    format!("node-fn-{path_string}{inputs_prefix}{inputs}{outputs_prefix}{outputs}")
}

/// The name of the graph fn compiled for the nested level at `path`, for the
/// variant invoked with the given active-input mask.
pub(crate) fn graph_fn_name(path: &[node::Id], inputs: &node::Conns) -> String {
    let path_string = path_string(path);
    let inputs_prefix = if inputs.is_empty() { "" } else { "-i" };
    format!("graph-fn-{path_string}{inputs_prefix}{inputs}")
}

/// The name of the fn evaluating one entrypoint's sources at the nested
/// level `path`.
pub(crate) fn lvl_fn_name(ep: &EntrypointId, path: &[node::Id]) -> String {
    format!("lvl-fn-{}-{}", ep.0.display_short(), path_string(path))
}

/// Generate entry fn name from an `EntrypointId`.
///
/// The name is deterministic and unique - derived from the content hash
/// (truncated to 8 hex chars).
pub fn entry_fn_name(id: &EntrypointId) -> String {
    format!("entry-fn-{}", id.0.display_short())
}

/// The emitted name of a variable.
pub(crate) fn var_name(var: &Var) -> String {
    match var {
        Var::Output { node, output } => format!("node-{node}-o{output}"),
        Var::Input { node, input } => format!("node-{node}-i{input}"),
        Var::Result { node } => pair_name(*node),
    }
}

/// The binding holding a branching node's raw `(branch-ix value)` result.
pub(crate) fn pair_name(node: node::Id) -> String {
    format!("node-{node}")
}

/// The emitted name of a join point fn.
pub(crate) fn join_name(join: JoinId) -> String {
    format!("join-{join}")
}

/// Parse an emitted identifier back to the entity it names.
///
/// Returns `None` for identifiers the compiler did not generate from a graph
/// entity (params, temporaries like `%vals-*`/`%lvl-*`, user identifiers).
pub fn parse(name: &str) -> Option<Name> {
    if let Some(rest) = name.strip_prefix("node-fn-") {
        let mut segs = rest.split('-');
        let path = parse_path(segs.next()?)?;
        let (inputs, outputs) = parse_io_conns(segs)?;
        Some(Name::NodeFn {
            path,
            inputs,
            outputs,
        })
    } else if let Some(rest) = name.strip_prefix("graph-fn-") {
        let mut segs = rest.split('-');
        let path = parse_path(segs.next()?)?;
        let inputs = match (segs.next(), segs.next()) {
            (None, _) => node::Conns::empty(),
            (Some(i), None) => parse_conns(i, 'i')?,
            _ => return None,
        };
        Some(Name::GraphFn { path, inputs })
    } else if let Some(rest) = name.strip_prefix("lvl-fn-") {
        let (ep, path) = rest.split_once('-')?;
        is_short_hash(ep).then_some(())?;
        let path = parse_path(path)?;
        Some(Name::LvlFn {
            ep: ep.to_string(),
            path,
        })
    } else if let Some(rest) = name.strip_prefix("entry-fn-") {
        is_short_hash(rest).then(|| Name::EntryFn {
            ep: rest.to_string(),
        })
    } else if let Some(rest) = name.strip_prefix("join-") {
        Some(Name::Join {
            id: parse_id(rest)?,
        })
    } else if let Some(rest) = name.strip_prefix("node-") {
        let Some((id, suffix)) = rest.split_once('-') else {
            return Some(Name::Result {
                node: parse_id(rest)?,
            });
        };
        let node = parse_id(id)?;
        if let Some(output) = suffix.strip_prefix('o') {
            Some(Name::Output {
                node,
                output: parse_id(output)?,
            })
        } else if let Some(input) = suffix.strip_prefix('i') {
            Some(Name::Input {
                node,
                input: parse_id(input)?,
            })
        } else {
            None
        }
    } else {
        None
    }
}

/// Parse a `:`-joined node path as formatted by [`path_string`].
fn parse_path(s: &str) -> Option<Vec<node::Id>> {
    s.split(':').map(parse_id).collect()
}

/// Parse a plain decimal id (rejecting signs, whitespace and empty strings).
fn parse_id(s: &str) -> Option<node::Id> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Parse a `i{conns}`/`o{conns}` segment with the given prefix.
fn parse_conns(s: &str, prefix: char) -> Option<node::Conns> {
    let bits = s.strip_prefix(prefix)?;
    if bits.is_empty() {
        return None;
    }
    bits.parse().ok()
}

/// Parse the optional `-i{conns}`/`-o{conns}` tail of a node fn name.
fn parse_io_conns(mut segs: std::str::Split<'_, char>) -> Option<(node::Conns, node::Conns)> {
    let empty = node::Conns::empty;
    match (segs.next(), segs.next(), segs.next()) {
        (None, _, _) => Some((empty(), empty())),
        (Some(i), None, _) if i.starts_with('i') => Some((parse_conns(i, 'i')?, empty())),
        (Some(o), None, _) if o.starts_with('o') => Some((empty(), parse_conns(o, 'o')?)),
        (Some(i), Some(o), None) => Some((parse_conns(i, 'i')?, parse_conns(o, 'o')?)),
        _ => None,
    }
}

/// Whether `s` is a truncated content-address hash as formatted by
/// `ContentAddr::display_short` (8 lowercase hex chars).
fn is_short_hash(s: &str) -> bool {
    s.len() == 8
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(byte: u8) -> EntrypointId {
        EntrypointId(gantz_ca::ContentAddr::from([byte; 32]))
    }

    fn conns(s: &str) -> node::Conns {
        s.parse().unwrap()
    }

    #[test]
    fn node_fn_roundtrip() {
        let cases = [
            (vec![0], conns(""), conns("")),
            (vec![0], conns("1"), conns("")),
            (vec![3], conns(""), conns("11")),
            (vec![0, 1, 2], conns("0101"), conns("1100")),
            (vec![10, 22, 333], conns("1"), conns("1")),
        ];
        for (path, inputs, outputs) in cases {
            let name = node_fn_name(&path, &inputs, &outputs);
            assert_eq!(
                parse(&name),
                Some(Name::NodeFn {
                    path,
                    inputs,
                    outputs
                }),
                "{name}"
            );
        }
    }

    #[test]
    fn graph_fn_roundtrip() {
        let cases = [
            (vec![0], conns("")),
            (vec![1, 4], conns("101")),
            (vec![12, 0], conns("1")),
        ];
        for (path, inputs) in cases {
            let name = graph_fn_name(&path, &inputs);
            assert_eq!(parse(&name), Some(Name::GraphFn { path, inputs }), "{name}");
        }
    }

    #[test]
    fn lvl_fn_roundtrip() {
        let id = ep(0xab);
        let path = vec![0, 7];
        let name = lvl_fn_name(&id, &path);
        assert_eq!(
            parse(&name),
            Some(Name::LvlFn {
                ep: "abababab".to_string(),
                path
            }),
            "{name}"
        );
    }

    #[test]
    fn entry_fn_roundtrip() {
        let id = ep(0x1f);
        let name = entry_fn_name(&id);
        assert_eq!(
            parse(&name),
            Some(Name::EntryFn {
                ep: "1f1f1f1f".to_string()
            }),
            "{name}"
        );
    }

    #[test]
    fn var_roundtrip() {
        let cases = [
            (
                Var::Output { node: 0, output: 0 },
                Name::Output { node: 0, output: 0 },
            ),
            (
                Var::Output {
                    node: 12,
                    output: 3,
                },
                Name::Output {
                    node: 12,
                    output: 3,
                },
            ),
            (
                Var::Input { node: 4, input: 1 },
                Name::Input { node: 4, input: 1 },
            ),
            (Var::Result { node: 9 }, Name::Result { node: 9 }),
        ];
        for (var, expected) in cases {
            let name = var_name(&var);
            assert_eq!(parse(&name), Some(expected), "{name}");
        }
    }

    #[test]
    fn join_roundtrip() {
        let name = join_name(42);
        assert_eq!(parse(&name), Some(Name::Join { id: 42 }), "{name}");
    }

    #[test]
    fn non_names_rejected() {
        let cases = [
            "",
            "state",
            "input0",
            "output",
            "results",
            "graph-state",
            "%root-state",
            "%lvl-state",
            "%lvl-r-3",
            "%gantz-sig",
            "%vals-node-0-o0",
            "'%gantz-unfired",
            // Malformed variants of real prefixes.
            "node-",
            "node-fn-",
            "node-fn-x",
            "node-fn-0:",
            "node-fn-0-z1",
            "node-fn-0-i",
            "node-0-",
            "node-0-x1",
            "node-1:2",
            "graph-fn-",
            "graph-fn-0-o1",
            "lvl-fn-0",
            "lvl-fn-zzzzzzzz-0",
            "lvl-fn-abcd-0",
            "entry-fn-xyz",
            "entry-fn-ABABABAB",
            "join-",
            "join-x",
        ];
        for name in cases {
            assert_eq!(parse(name), None, "{name}");
        }
    }
}
