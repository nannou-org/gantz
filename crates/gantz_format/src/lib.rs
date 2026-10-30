//! A human-readable text format for gantz graph registries.
//!
//! `gantz_format` is the layout-agnostic core of the `.gantz` format. It reads
//! and writes a [`gantz_ca::Registry`] of graphs as S-expression text. The text
//! is reader-valid Steel, so embedded node code needs no escaping and tooling
//! can reuse Steel's reader. The author never needs to know a content address.
//!
//! It recognises only the registry forms `(graph ...)`, `(commits ...)` and
//! `(names ...)`. Unrecognised top-level forms are preserved, not errored. See
//! [`Form`]. Richer layers can extend the format with their own forms, for
//! example a GUI `(layout ...)` form. They use the [`sexpr`] toolkit and the
//! resolution context returned by [`from_str_normalized`] and
//! [`to_string_with`].
//!
//! Node keywords such as `expr` and `inlet` are pluggable [`Sugar`]. Each
//! crate owns the sugar for its own nodes, and a node set composes them into
//! one [`Sugars`]. [`CoreSugar`] covers `gantz_core`'s nodes. A node that no sugar recognises round-trips
//! through the generic `(node ...)` form.
//!
//! Reading takes a [`Normalize`] fn, which turns each parsed node into its
//! canonical stored [`gantz_ca::NodeData`]. A node set's codec supplies it.
//! The registry stores graphs in their erased data form,
//! [`gantz_ca::DataGraph`]. See [`gantz_core::data`]. Writing needs no node
//! type at all. The stored [`gantz_ca::NodeData`] is already the tagged form
//! the writer consumes, so [`to_string_with`] and [`to_string_named_with`]
//! serialize any registry without the node set compiled in.

mod datum;
mod error;
mod lower;
mod model;
mod parse;
mod raise;
mod sugar;
mod writer;

pub mod sexpr;

pub use datum::{datum_from_expr, datum_text};
pub use error::{ErrorKind, FormatError, Span, line_col};
#[doc(inline)]
pub use gantz_core::datum::{Datum, DatumError, from_datum, node_datum, to_datum};
pub use lower::{Loaded, Normalize};
pub use model::{Addr, Document, Form};
pub use raise::{Dumped, GraphLabels};
pub use sugar::{CoreSugar, Sugar, SugarArgs, Sugars};

use gantz_ca::{Registry, Timestamp};

/// Parse a `.gantz` document into its [`Loaded`] registry, resolution context
/// and preserved extra forms.
///
/// `sugar` reads the node keywords. `normalize` turns each parsed node into
/// its canonical stored form. See [`Normalize`]. `now` provides the timestamp
/// for any graph the `(commits ...)` table does not describe. Hand-authored
/// graphs often have no commit entry.
///
/// Names the document does not define resolve through `seed`, which maps a
/// known name to a head graph address. This lets a document reference graphs
/// defined elsewhere. For example, a domain's base source can reference
/// another source's graphs. The document's own names shadow the seed. A
/// seeded reference embeds the seeded graph address in the built node, so the
/// referring graph's content address depends on it. Callers that want
/// reproducible addresses must seed reproducible ones. Pass an empty `seed`
/// to resolve only the document's own names.
pub fn from_str_normalized(
    text: &str,
    now: Timestamp,
    sugar: &dyn Sugar,
    seed: &std::collections::BTreeMap<String, gantz_ca::GraphAddr>,
    normalize: &Normalize,
) -> Result<Loaded, FormatError> {
    let doc = parse::parse(text, sugar)?;
    lower::lower(doc, now, seed, normalize)
}

/// Serialize a registry to `.gantz` text using the given keyword [`Sugar`].
/// Returns the text along with the per-graph label context an extender needs
/// to emit its own forms.
///
/// Metadata sections are written as generic `(section ...)` forms, except
/// the ids in `claimed`. The caller renders those itself with friendly forms
/// such as `(descriptions ...)`.
pub fn to_string_with(
    registry: &Registry,
    sugar: &dyn Sugar,
    claimed: &[&str],
) -> Result<Dumped, FormatError> {
    raise::raise(registry, sugar, claimed)
}

/// Serialize a registry in the inline-name format. Each named graph is emitted
/// under its registry name, with no `(commits ...)` or `(names ...)` tables.
/// References resolve by name. Intended for hand-editable, churn-free files
/// such as the baked-in base. See [`to_string_with`] for `claimed`.
pub fn to_string_named_with(
    registry: &Registry,
    sugar: &dyn Sugar,
    claimed: &[&str],
) -> Result<Dumped, FormatError> {
    raise::raise_named(registry, sugar, claimed)
}

#[cfg(test)]
mod test_util {
    use crate::{Datum, FormatError};
    use gantz_ca::NodeData;
    use gantz_nodetag::NodeTag;

    /// A [`crate::Normalize`] for a node set of the one type `T`. It decodes
    /// the parsed fields at `T` and erases the node back to canonical data.
    pub(crate) fn normalize_as<T>(datum: Datum) -> Result<NodeData, FormatError>
    where
        T: NodeTag + serde::Serialize + serde::de::DeserializeOwned + gantz_core::Node,
    {
        let tag = datum.get("type").and_then(Datum::as_str).unwrap_or("?");
        let err = |e: String| FormatError::node_deserialize(tag.to_string(), e);
        if tag != T::TAG {
            return Err(err(format!("not a `{}`", T::TAG)));
        }
        let Datum::Map(mut fields) = datum.clone() else {
            return Err(err("not a map".to_string()));
        };
        fields.retain(|(k, _)| k != "type");
        let node_data = NodeData::new(T::TAG, Datum::Map(fields));
        let node: T =
            gantz_core::data::reify_node_concrete(&node_data).map_err(|e| err(e.to_string()))?;
        gantz_core::data::erase_node_typed(&node).map_err(|e| err(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    //! A `Sugar` is entirely optional for a node type. A node whose tag no
    //! sugar recognises round-trips through the generic
    //! `(node "Tag" ...)` form. This guards that property.

    use super::test_util::normalize_as;
    use super::*;
    use gantz_core::Node;
    use gantz_nodetag::NodeTag;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    // A node type with no `Sugar`. It carries no first-class keyword at all.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, NodeTag)]
    struct Knob {
        value: i64,
    }

    impl Node for Knob {
        fn expr(&self, _: gantz_core::node::ExprCtx) -> gantz_core::node::ExprResult {
            unimplemented!("not compiled in these tests")
        }
    }

    fn parse(text: &str) -> Result<Loaded, FormatError> {
        let now = std::time::Duration::ZERO;
        from_str_normalized(
            text,
            now,
            &CoreSugar,
            &BTreeMap::new(),
            &normalize_as::<Knob>,
        )
    }

    #[test]
    fn a_node_without_sugar_round_trips_in_the_generic_form() {
        // A graph with one node written in the generic form. `Knob` is unknown to
        // every sugar, so it must round-trip via `(node "Knob" ...)`.
        let text = "(graph g (k (node \"Knob\" (value 7))))";
        let loaded = parse(text).expect("parse without sugar");
        let dumped =
            to_string_with(&loaded.registry, &CoreSugar, &[]).expect("write without sugar");

        assert!(
            dumped.text.contains("(node \"Knob\""),
            "expected generic node form, got:\n{}",
            dumped.text,
        );
        assert!(dumped.text.contains("(value 7)"));

        // The reparse is stable.
        let reloaded = parse(&dumped.text).expect("reparse");
        assert_eq!(
            to_string_with(&reloaded.registry, &CoreSugar, &[])
                .expect("rewrite")
                .text,
            dumped.text,
        );
    }

    /// Merge commits round-trip their extra parents via the additive
    /// `(merge-parents ...)` clause. Ordinary commits are written without it.
    #[test]
    fn merge_parents_round_trip() {
        let text = "\
            (graph g1 (k (node \"Knob\" (value 1))))\n\
            (graph g2 (k (node \"Knob\" (value 2))))\n\
            (graph g3 (k (node \"Knob\" (value 3))))\n\
            (commits\n\
              (c1 (time 1 0) (graph g1))\n\
              (c2 (time 2 0) (graph g2))\n\
              (c3 (time 3 0) (parent c1) (merge-parents c2) (graph g3)))";
        let loaded = parse(text).expect("parse merge commit");
        let merge = loaded
            .registry
            .commits()
            .values()
            .find(|c| !c.merge_parents.is_empty())
            .expect("merge commit survives the parse");
        assert!(merge.parent.is_some());
        assert_eq!(merge.merge_parents.len(), 1);
        assert_ne!(merge.parent, Some(merge.merge_parents[0]));

        // The merge parent survives a write and reparse. Non-merge commits
        // carry no `merge-parents` clause.
        let dumped = to_string_with(&loaded.registry, &CoreSugar, &[]).expect("write merge commit");
        assert_eq!(dumped.text.matches("(merge-parents").count(), 1);
        let reloaded = parse(&dumped.text).expect("reparse");
        let re_merge = reloaded
            .registry
            .commits()
            .values()
            .find(|c| !c.merge_parents.is_empty())
            .expect("merge commit survives the round-trip");
        assert_eq!(re_merge.merge_parents.len(), 1);
    }
}

#[cfg(test)]
mod data_only_tests {
    //! Raising is node-set-free. A registry built purely from [`NodeData`]
    //! literals serializes to text with no node type compiled in. This lets a
    //! relay or tool that depends only on `gantz_ca` and `gantz_format` export
    //! a registry.

    use super::*;
    use gantz_ca::{Commit, DataGraph, Datum, Edge, NodeData};

    #[test]
    fn raising_needs_no_node_set() {
        let mut graph = DataGraph::default();
        let expr = graph.add_node(NodeData::new(
            "Expr",
            Datum::Map(vec![("src".to_string(), Datum::Str("(+ 1 2)".to_string()))]),
        ));
        let gain = graph.add_node(NodeData::new(
            "Gain",
            Datum::Map(vec![("db".to_string(), Datum::I64(6))]),
        ));
        graph.add_edge(expr, gain, Edge::from((0u16, 0u16)));

        let mut registry: Registry = Registry::default();
        let g_addr = registry.add_graph(graph);
        let c_addr = registry.add_commit(Commit::new(Timestamp::ZERO, None, g_addr));
        registry.set_head("patch".parse().expect("valid name"), c_addr);

        let dumped = to_string_with(&registry, &CoreSugar, &[]).expect("raise from data alone");
        let g = &gantz_ca::ContentAddr::from(g_addr).to_string()[..8];
        let c = &gantz_ca::ContentAddr::from(c_addr).to_string()[..8];
        let expected = format!(
            "(graph \"{g}\"\n\
            \x20 (expr0 (expr (+ 1 2)))\n\
            \x20 (node1 (node \"Gain\" (db 6)))\n\
            \x20 (-> expr0 node1))\n\
            \n\
            (commits\n\
            \x20 (\"{c}\" (time 0 0) (graph \"{g}\")))\n\
            \n\
            (names\n\
            \x20 (patch \"{c}\"))\n",
        );
        assert_eq!(dumped.text, expected);
    }
}

#[cfg(test)]
mod seed_tests {
    use super::test_util::normalize_as;
    use super::*;
    use gantz_core::Node;
    use gantz_nodetag::NodeTag;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    // A ref node that matches the wire tag the format's ref lowering produces.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, NodeTag)]
    #[tag("NamedRef")]
    struct TestRef {
        ref_: gantz_core::node::Ref,
        name: String,
        #[serde(default)]
        sync: bool,
    }

    impl Node for TestRef {
        fn expr(&self, _: gantz_core::node::ExprCtx) -> gantz_core::node::ExprResult {
            unimplemented!("not compiled in these tests")
        }

        fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
            vec![self.ref_.content_addr()]
        }
    }

    fn parse(
        text: &str,
        seed: &BTreeMap<String, gantz_ca::GraphAddr>,
    ) -> Result<Loaded, FormatError> {
        let now = std::time::Duration::ZERO;
        from_str_normalized(text, now, &CoreSugar, seed, &normalize_as::<TestRef>)
    }

    fn seed_graph() -> gantz_ca::GraphAddr {
        gantz_ca::GraphAddr::from(gantz_ca::ContentAddr::from([7u8; 32]))
    }

    fn ref_addr_of(loaded: &Loaded, name: &str) -> gantz_ca::ContentAddr {
        let commit = loaded.names[name];
        let data_graph = loaded
            .registry
            .commit_graph_ref(&commit)
            .expect("graph for name");
        let node: TestRef = data_graph
            .node_weights()
            .find_map(|nd| gantz_core::data::reify_node_concrete(nd).ok())
            .expect("a ref node");
        node.ref_.content_addr()
    }

    /// A reference to a name the document does not define fails with
    /// `MissingDependency` unseeded, and resolves to the seeded graph
    /// address when seeded.
    #[test]
    fn seed_resolves_foreign_names() {
        let text = "(graph use-foreign (r (ref foreign)))";

        let err = match parse(text, &BTreeMap::new()) {
            Err(e) => e,
            Ok(_) => panic!("must not resolve"),
        };
        assert!(
            matches!(&err.kind, ErrorKind::MissingDependency(name) if name == "foreign"),
            "unexpected error: {err:?}",
        );

        let seed = [("foreign".to_string(), seed_graph())]
            .into_iter()
            .collect();
        let loaded = parse(text, &seed).expect("seeded parse");
        assert_eq!(
            ref_addr_of(&loaded, "use-foreign"),
            gantz_ca::ContentAddr::from(seed_graph()),
            "the built ref must embed the seeded graph address",
        );
    }

    /// The pinned-address arm heals through the seed too. A stale pinned
    /// address whose name only the seed knows resolves to the seeded graph.
    #[test]
    fn seed_heals_stale_pinned_addr() {
        let stale = "ee".repeat(32);
        let text = format!("(graph use-foreign (r (ref foreign \"{stale}\")))");

        let err = match parse(&text, &BTreeMap::new()) {
            Err(e) => e,
            Ok(_) => panic!("must not resolve"),
        };
        assert!(matches!(&err.kind, ErrorKind::MissingDependency(_)));

        let seed = [("foreign".to_string(), seed_graph())]
            .into_iter()
            .collect();
        let loaded = parse(&text, &seed).expect("seeded parse");
        assert_eq!(
            ref_addr_of(&loaded, "use-foreign"),
            gantz_ca::ContentAddr::from(seed_graph()),
        );
    }
}
