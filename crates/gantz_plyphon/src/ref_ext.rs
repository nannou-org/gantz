//! The DSP domain's `Ref` extension: the per-reference `inline` flag (#295),
//! and the data-level DSP-graph discovery backing its inspector toggle.
//!
//! [`DspRefExt`] is stored in the referenced node's ext slot (see
//! [`Ref::set_ext`](gantz_core::node::Ref::set_ext)) under
//! [`DSP_REF_EXT_KEY`], only when non-default so a default-configured
//! reference keeps its address. `crate::egui`'s `DspRefExtUi` (`egui` feature)
//! renders the inspector toggle for references whose graph contains DSP
//! nodes.

use gantz_ca::{ContentAddr, GraphAddr, Registry};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// The DSP domain's per-reference options.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DspRefExt {
    /// Inline the referenced graph's DSP nodes into the parent synthdef,
    /// rather than instancing a shared definition.
    ///
    /// Instancing is the default lowering: the child derives once into shared
    /// content-named synthdefs, installed once and spawned per instance with
    /// bus wiring set per synth. Inlining splices the child's units into the
    /// parent's def instead - each copy compiles its own units, buying full
    /// cross-boundary fusion at N-times the unit count.
    pub inline: bool,
}

/// The ext key under which [`DspRefExt`] is stored.
pub const DSP_REF_EXT_KEY: &str = "plyphon.dsp-ref";

/// The wire tags of this crate's DSP node types: the data-level equivalent of
/// the [`ToNodeDsp`](crate::ToNodeDsp) downcast probe (every type
/// [`node_dsp_of`](crate::node_dsp_of) matches, each of which is
/// unconditionally DSP). Sourced from the types' [`NodeTag`] consts so a
/// renamed tag cannot drift. Keep in step with
/// [`node_dsp_of`](crate::node_dsp_of).
const DSP_NODE_TAGS: [&str; 8] = [
    crate::Bus::TAG,
    crate::Out::TAG,
    crate::Pack::TAG,
    crate::PlayBuf::TAG,
    crate::ScopeOut::TAG,
    crate::Sum::TAG,
    crate::UnitNode::TAG,
    crate::Unpack::TAG,
];

/// The wire tag of `gantz_egui`'s `NamedRef` wrapper.
///
/// A literal rather than `NamedRef::TAG` so the headless build (no
/// `gantz_egui` in the tree) still recognises stored references. The tag is
/// already a cross-crate wire convention (`gantz_format`'s raise/lower match
/// it at the data level); the `egui`-gated test below pins this constant to
/// the type's own `NodeTag` so a renamed tag cannot drift silently.
const NAMED_REF_TAG: &str = "NamedRef";

/// The wire tags of reference-transparent nodes: the data-level equivalent of
/// the [`AsRefNode`](gantz_core::node::AsRefNode) probe. A bare
/// [`Ref`](gantz_core::node::Ref) or a `NamedRef` wrapper stands in for the
/// graph it references, so DSP-ness flows through it. `FnNamedRef` is
/// deliberately absent: a function value references a graph without standing
/// in for it (mirroring its missing `AsRefNode` impl).
const REF_NODE_TAGS: [&str; 2] = [gantz_core::node::Ref::TAG, NAMED_REF_TAG];

/// The graph addresses (as [`ContentAddr`]s, the form
/// [`Ref::content_addr`](gantz_core::node::Ref::content_addr) reports) of
/// every registry graph containing DSP nodes, directly or transitively
/// through references.
///
/// A pure walk over the stored [`DataGraph`](gantz_ca::DataGraph)s: DSP nodes
/// are recognised by wire tag and references are followed through their
/// [`NodeData`](gantz_ca::NodeData) `refs` column (a reference node's one ref
/// is its target, per `Node::required_addrs`), so no typed node set or
/// reified cache is required. Addresses that do not resolve to registry
/// graphs classify as non-DSP.
pub fn dsp_graphs(registry: &Registry) -> HashSet<ContentAddr> {
    let mut memo: HashMap<GraphAddr, bool> = HashMap::new();
    let mut stack: Vec<GraphAddr> = Vec::new();
    registry
        .graphs()
        .keys()
        .copied()
        .filter(|&ga| is_dsp(registry, ga, &mut memo, &mut stack))
        .map(ContentAddr::from)
        .collect()
}

/// Whether the registry graph at `ga` contains a DSP node, directly or
/// transitively through references. See [`dsp_graphs`] for how the stored
/// data is classified.
pub fn is_dsp_graph(registry: &Registry, ga: &GraphAddr) -> bool {
    is_dsp(registry, *ga, &mut HashMap::new(), &mut Vec::new())
}

/// Whether the graph at `ga` contains a DSP node, directly or transitively
/// through references. Memoized per graph so repeated probes over one
/// registry stay linear overall; reference cycles are treated as non-DSP at
/// the point of re-entry (a cycle cannot introduce a DSP node that its
/// members do not already contain).
fn is_dsp(
    registry: &Registry,
    ga: GraphAddr,
    memo: &mut HashMap<GraphAddr, bool>,
    stack: &mut Vec<GraphAddr>,
) -> bool {
    if let Some(&known) = memo.get(&ga) {
        return known;
    }
    if stack.contains(&ga) {
        return false;
    }
    let Some(graph) = registry.graph(&ga) else {
        memo.insert(ga, false);
        return false;
    };
    stack.push(ga);
    let dsp = graph
        .node_weights()
        .any(|n| DSP_NODE_TAGS.contains(&n.tag.as_str()))
        || graph.node_weights().any(|n| {
            REF_NODE_TAGS.contains(&n.tag.as_str())
                && n.refs
                    .iter()
                    .any(|&addr| is_dsp(registry, addr.into(), memo, stack))
        });
    stack.pop();
    memo.insert(ga, dsp);
    dsp
}

#[cfg(all(test, feature = "egui"))]
mod tests {
    use super::*;

    /// Pin the headless literal to the type's own wire tag: a `NamedRef` tag
    /// rename must show up here, not as silently-broken discovery.
    #[test]
    fn named_ref_tag_matches_node_tag() {
        assert_eq!(NAMED_REF_TAG, gantz_egui::node::NamedRef::TAG);
    }
}
