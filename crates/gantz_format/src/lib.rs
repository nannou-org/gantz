//! A human-readable text format for gantz graph registries.
//!
//! `gantz_format` is the layout-agnostic core of the `.gantz` format: it reads
//! and writes a [`gantz_ca::Registry`] of graphs as S-expression text that is
//! reader-valid Steel (so embedded node code needs no escaping and tooling can
//! reuse Steel's reader), without requiring the author to know any content
//! addresses.
//!
//! It recognises only the registry forms - `(graph ...)`, `(commits ...)` and
//! `(names ...)`. Unrecognised top-level forms are preserved (see [`Form`]),
//! not errored, so richer layers can extend the format - e.g. a GUI adding
//! `(layout ...)` - using the [`sexpr`] toolkit together with the resolution
//! context returned by [`from_str`]/[`to_string`].
//!
//! Node keywords (`expr`, `inlet`, ...) are pluggable [`Sugar`]: [`from_str`]
//! and [`to_string`] read the node set's composite via [`NodeSugar`]
//! (`N::sugar()`), so each crate owns the sugar for its own nodes ([`CoreSugar`]
//! covers `gantz_core`'s). The `_with` variants accept any `&dyn Sugar`
//! explicitly (compose with [`Sugars`]), still falling back to the generic
//! `(node ...)` form. On the reading side any node type that is `Serialize +
//! DeserializeOwned + gantz_core::Node` works: the registry stores graphs in
//! their erased data form ([`gantz_ca::DataGraph`]), and the node-set type is
//! the codec parsed text travels through (see [`gantz_core::data`]). The
//! writing side needs no node type at all: the stored [`gantz_ca::NodeData`]
//! (`tag` + field datum) is already the tagged form the writer consumes, so
//! [`to_string_with`]/[`to_string_named_with`] can serialize any registry
//! without the node set compiled in.

mod datum;
mod error;
mod lower;
mod model;
mod node_set;
mod parse;
mod raise;
mod sugar;
mod writer;

pub mod sexpr;

pub use datum::{datum_from_expr, datum_text};
pub use error::{ErrorKind, FormatError, Span};
#[doc(inline)]
pub use gantz_core::datum::{Datum, DatumError, from_datum, node_datum, to_datum};
pub use lower::{Loaded, Normalize};
pub use model::{Addr, Document, Form};
#[doc(hidden)]
pub use node_set::{NodeFields, TaggedNode};
pub use raise::{Dumped, GraphLabels};
pub use sugar::{CoreSugar, NodeSugar, Sugar, SugarArgs, Sugars};

/// Re-exported for [`impl_node_set_serde!`] expansions (`$crate::NodeTag`);
/// depend on `gantz_nodetag` directly to implement or derive it.
#[doc(hidden)]
pub use gantz_nodetag::NodeTag;

use gantz_ca::{Registry, Timestamp};
use gantz_core::Node;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Parse a `.gantz` document (using the node set's composite [`NodeSugar`]) into
/// its [`Loaded`] registry, resolution context and preserved extra forms.
///
/// `now` provides the timestamp for any graph the `(commits ...)` table does not
/// describe (hand-authored graphs with no commit entry).
pub fn from_str<N>(text: &str, now: Timestamp) -> Result<Loaded, FormatError>
where
    N: Serialize + DeserializeOwned + Node + NodeSugar,
{
    from_str_with::<N>(text, now, &N::sugar())
}

/// Parse a `.gantz` document using a custom keyword [`Sugar`] (compose with
/// [`CoreSugar`] via [`Sugars`] to keep `gantz_core`'s built-ins).
pub fn from_str_with<N>(
    text: &str,
    now: Timestamp,
    sugar: &dyn Sugar,
) -> Result<Loaded, FormatError>
where
    N: Serialize + DeserializeOwned + Node,
{
    let doc = parse::parse(text, sugar)?;
    lower::lower::<N>(doc, now)
}

/// [`from_str`], resolving names the document does not define through `seed`
/// (externally-known name -> head graph associations).
///
/// Lets a document reference graphs defined elsewhere, e.g. a domain's base
/// source referencing another source's graphs. The document's own names
/// shadow the seed. Note a seeded reference embeds the seeded graph address
/// in the built node, so the referring graph's content address depends on
/// it - callers wanting reproducible addresses must seed reproducible ones.
pub fn from_str_seeded<N>(
    text: &str,
    now: Timestamp,
    seed: &std::collections::BTreeMap<String, gantz_ca::GraphAddr>,
) -> Result<Loaded, FormatError>
where
    N: Serialize + DeserializeOwned + Node + NodeSugar,
{
    let doc = parse::parse(text, &N::sugar())?;
    lower::lower_seeded::<N>(doc, now, seed)
}

/// Parse a `.gantz` document with an explicit keyword [`Sugar`] and node
/// [`Normalize`] seam in place of a node-set type parameter.
///
/// The node-set-free counterpart of [`from_str_seeded`], for callers whose
/// node set is a value-level codec rather than a serde-dispatching type
/// (pass an empty `seed` for the plain [`from_str`] behaviour).
pub fn from_str_normalized(
    text: &str,
    now: Timestamp,
    sugar: &dyn Sugar,
    seed: &std::collections::BTreeMap<String, gantz_ca::GraphAddr>,
    normalize: &Normalize,
) -> Result<Loaded, FormatError> {
    let doc = parse::parse(text, sugar)?;
    lower::lower_normalized(doc, now, seed, normalize)
}

/// Serialize a registry to `.gantz` text (with gantz's built-in node keywords),
/// returning the text along with the per-graph label context an extender needs
/// to emit its own forms.
///
/// Metadata sections are written as generic `(section ...)` forms, except
/// the ids in `claimed`, which the caller renders itself with friendly
/// forms (e.g. `(descriptions ...)`).
pub fn to_string<N>(registry: &Registry, claimed: &[&str]) -> Result<Dumped, FormatError>
where
    N: NodeSugar,
{
    to_string_with(registry, &N::sugar(), claimed)
}

/// Serialize a registry to `.gantz` text using a custom keyword [`Sugar`].
pub fn to_string_with(
    registry: &Registry,
    sugar: &dyn Sugar,
    claimed: &[&str],
) -> Result<Dumped, FormatError> {
    raise::raise(registry, sugar, claimed)
}

/// Serialize a registry in the inline-name format: each named graph is emitted
/// under its registry name, with no `(commits ...)` / `(names ...)` tables and
/// references resolved by name. Intended for hand-editable, churn-free files
/// such as the baked-in base. See [`to_string`] for `claimed`.
pub fn to_string_named<N>(registry: &Registry, claimed: &[&str]) -> Result<Dumped, FormatError>
where
    N: NodeSugar,
{
    to_string_named_with(registry, &N::sugar(), claimed)
}

/// As [`to_string_named`], with a custom keyword [`Sugar`].
pub fn to_string_named_with(
    registry: &Registry,
    sugar: &dyn Sugar,
    claimed: &[&str],
) -> Result<Dumped, FormatError> {
    raise::raise_named(registry, sugar, claimed)
}

#[cfg(test)]
mod tests {
    //! `NodeSugar` and `Sugar` are both entirely optional for a downstream
    //! node-set type. The `_with` entry points carry no `NodeSugar` bound, and a
    //! node whose tag no sugar recognises simply round-trips through the generic
    //! `(node "Tag" ...)` form. This guards that property.

    use super::*;
    use gantz_nodetag::NodeTag;
    use serde::{Deserialize, Serialize};

    // A self-contained node-set with one node type that implements neither
    // `NodeSugar` nor any `Sugar` - it carries no first-class keyword at all.
    trait Widget: std::any::Any + Node {}

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, NodeTag)]
    struct Knob {
        value: i64,
    }

    impl Node for Knob {
        fn expr(&self, _: gantz_core::node::ExprCtx) -> gantz_core::node::ExprResult {
            unimplemented!("not compiled in these tests")
        }
    }

    impl Widget for Knob {}

    // `Box<dyn Widget>` is the node-set type `N`: `impl_node_set_serde!`
    // supplies its Serialize/Deserialize, and `gantz_core`'s blanket
    // `Node for Box<T>` covers the rest. It implements no `NodeSugar`.
    crate::impl_node_set_serde! {
        dyn Widget {
            Knob,
        }
    }

    #[test]
    fn the_with_variants_need_no_node_sugar() {
        // A graph with one node written in the generic form. `Knob` is unknown to
        // every sugar, so it must round-trip via `(node "Knob" ...)`.
        let text = "(graph g (k (node \"Knob\" (value 7))))";

        // Both `_with` calls compile and run even though `Box<dyn Widget>`
        // implements neither `NodeSugar` nor `Sugar`. (The convenience
        // `from_str`/`to_string` would instead require a `NodeSugar` impl.)
        let loaded = from_str_with::<Box<dyn Widget>>(text, std::time::Duration::ZERO, &CoreSugar)
            .expect("parse without NodeSugar");
        let dumped =
            to_string_with(&loaded.registry, &CoreSugar, &[]).expect("write without NodeSugar");

        // The node survived the round-trip through the generic form.
        assert!(
            dumped.text.contains("(node \"Knob\""),
            "expected generic node form, got:\n{}",
            dumped.text,
        );
        assert!(dumped.text.contains("(value 7)"));

        // And the reparse is stable.
        let reloaded =
            from_str_with::<Box<dyn Widget>>(&dumped.text, std::time::Duration::ZERO, &CoreSugar)
                .expect("reparse");
        assert_eq!(
            to_string_with(&reloaded.registry, &CoreSugar, &[])
                .expect("rewrite")
                .text,
            dumped.text,
        );
    }

    /// Merge commits round-trip their extra parents via the additive
    /// `(merge-parents ...)` clause; ordinary commits are written without it.
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
        let loaded = from_str_with::<Box<dyn Widget>>(text, std::time::Duration::ZERO, &CoreSugar)
            .expect("parse merge commit");
        let merge = loaded
            .registry
            .commits()
            .values()
            .find(|c| !c.merge_parents.is_empty())
            .expect("merge commit survives the parse");
        assert!(merge.parent.is_some());
        assert_eq!(merge.merge_parents.len(), 1);
        assert_ne!(merge.parent, Some(merge.merge_parents[0]));

        // The merge parent survives a write + reparse; non-merge commits carry
        // no `merge-parents` clause.
        let dumped = to_string_with(&loaded.registry, &CoreSugar, &[]).expect("write merge commit");
        assert_eq!(dumped.text.matches("(merge-parents").count(), 1);
        let reloaded =
            from_str_with::<Box<dyn Widget>>(&dumped.text, std::time::Duration::ZERO, &CoreSugar)
                .expect("reparse");
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
    //! Raising is node-set-free: a registry built purely from [`NodeData`]
    //! literals - no node type compiled in at all - serializes to text. This is
    //! what lets a relay or tool depending only on `gantz_ca` + `gantz_format`
    //! export a registry.

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
    use super::*;
    use gantz_nodetag::NodeTag;
    use serde::{Deserialize, Serialize};

    // A ref-capable node set: one type matching the wire tag the format's
    // ref lowering produces.
    trait RefNode: std::any::Any + Node {}

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

    impl RefNode for TestRef {}

    crate::impl_node_set_serde! {
        dyn RefNode {
            TestRef,
        }
    }

    impl NodeSugar for Box<dyn RefNode> {
        fn sugar() -> Sugars<'static> {
            Sugars(vec![&CoreSugar])
        }
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
        let graph: gantz_core::node::graph::Graph<Box<dyn RefNode>> =
            gantz_core::data::reify(data_graph).expect("reify through the node set");
        let node = graph
            .node_indices()
            .find_map(|ix| (&*graph[ix] as &dyn std::any::Any).downcast_ref::<TestRef>())
            .expect("a ref node");
        node.ref_.content_addr()
    }

    /// A reference to a name the document does not define fails with
    /// `MissingDependency` unseeded, and resolves to the seeded graph
    /// address when seeded.
    #[test]
    fn seed_resolves_foreign_names() {
        let text = "(graph use-foreign (r (ref foreign)))";
        let now = std::time::Duration::ZERO;

        let err = match from_str::<Box<dyn RefNode>>(text, now) {
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
        let loaded = from_str_seeded::<Box<dyn RefNode>>(text, now, &seed).expect("seeded parse");
        assert_eq!(
            ref_addr_of(&loaded, "use-foreign"),
            gantz_ca::ContentAddr::from(seed_graph()),
            "the built ref must embed the seeded graph address",
        );
    }

    /// The pinned-address arm heals through the seed too: a stale pinned
    /// address whose name only the seed knows resolves to the seeded graph.
    #[test]
    fn seed_heals_stale_pinned_addr() {
        let stale = "ee".repeat(32);
        let text = format!("(graph use-foreign (r (ref foreign \"{stale}\")))");
        let now = std::time::Duration::ZERO;

        let err = match from_str::<Box<dyn RefNode>>(&text, now) {
            Err(e) => e,
            Ok(_) => panic!("must not resolve"),
        };
        assert!(matches!(&err.kind, ErrorKind::MissingDependency(_)));

        let seed = [("foreign".to_string(), seed_graph())]
            .into_iter()
            .collect();
        let loaded = from_str_seeded::<Box<dyn RefNode>>(&text, now, &seed).expect("seeded parse");
        assert_eq!(
            ref_addr_of(&loaded, "use-foreign"),
            gantz_ca::ContentAddr::from(seed_graph()),
        );
    }
}
