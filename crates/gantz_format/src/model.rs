//! The abstract syntax for the `.gantz` text format.
//!
//! A [`Document`] is the intermediate representation between the reader in
//! [`super::parse`] and the content-addressed registry in [`super::lower`] and
//! [`super::raise`]. It mirrors the registry's three maps. Those are graph
//! bodies, a `(commits ...)` table and a `(names ...)` table. It preserves any
//! unrecognised top-level forms as [`Form`]s for extenders.

use crate::datum::Datum;
use crate::error::Span;

/// A parsed `.gantz` document.
#[derive(Clone, Debug, Default)]
pub struct Document {
    /// Graph bodies, in source order.
    pub graphs: Vec<GraphDef>,
    /// The flat commit table. At most one head commit per graph.
    pub commits: Vec<CommitDecl>,
    /// Name to commit mappings.
    pub names: Vec<NameDecl>,
    /// Generic metadata sections, the `(section ...)` forms.
    pub sections: Vec<SectionForm>,
    /// Unrecognised top-level forms, preserved verbatim for extenders.
    pub extra: Vec<Form>,
}

/// An unrecognised top-level form, preserved for an extender to interpret.
#[derive(Clone, Debug)]
pub struct Form {
    /// The form's head keyword, for example `"layout"`.
    pub head: String,
    /// The form's verbatim source text. Parse it with [`crate::sexpr::read`].
    pub raw: String,
    /// The form's source span in the original document.
    pub span: Span,
}

/// A graph body, identified by a file-local id.
#[derive(Clone, Debug)]
pub struct GraphDef {
    /// The graph's file-local id. It is a concrete graph address string or a
    /// label symbol. A label that no `(commits ...)` entry references is
    /// treated as a registry name with a synthesised root commit. This is the
    /// hand-authoring path.
    pub id: Addr,
    /// The graph interior.
    pub body: GraphBody,
}

/// The interior of a graph. Node declarations in index order, plus connections.
#[derive(Clone, Debug, Default)]
pub struct GraphBody {
    /// Node declarations. Declaration order is the node index by default.
    pub nodes: Vec<NodeDecl>,
    /// Connections between node ports.
    pub conns: Vec<Conn>,
}

/// A single node declaration within a graph.
#[derive(Clone, Debug)]
pub struct NodeDecl {
    /// File-local label, referenced by connections and layout.
    pub name: String,
    /// The node specification.
    pub spec: NodeSpec,
}

/// A node specification.
#[derive(Clone, Debug)]
pub enum NodeSpec {
    /// A self-contained node as a serde [`Datum`] map with a `type` field and
    /// its fields.
    Value(Datum),
    /// A `NamedRef` or `FnNamedRef` whose address resolves at load time.
    ///
    /// Nested graphs are not inlined. They are ordinary named graphs in the
    /// registry, referenced here like any other named graph.
    Ref(RefSpec),
}

/// A reference to another graph by name.
#[derive(Clone, Debug)]
pub struct RefSpec {
    /// `true` for `fn-ref`, which builds a `FnNamedRef`. `false` for `ref`,
    /// which builds a `NamedRef`.
    pub func: bool,
    /// The referenced name.
    pub name: String,
    /// Optional pinned commit. `None` resolves to the name's head commit.
    pub addr: Option<Addr>,
    /// Whether the reference should track the latest commit.
    pub sync: bool,
    /// Optional domain-extension data carried by the reference. It is a datum
    /// map keyed by domain, the text form of `gantz_core::node::Ref`'s ext.
    /// `None` when the reference carries no extension data.
    pub ext: Option<Datum>,
}

/// A file-local address token. Either a concrete content address or a label.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Addr {
    /// A concrete content address as a hex string, full or an unambiguous
    /// prefix.
    Concrete(String),
    /// A file-local label symbol, resolved to a computed address on load.
    Label(String),
}

/// A connection between two node ports.
#[derive(Clone, Debug)]
pub struct Conn {
    /// The source endpoint.
    pub from: Endpoint,
    /// The destination endpoint.
    pub to: Endpoint,
}

/// One end of a connection. A node label and a port index.
#[derive(Clone, Debug)]
pub struct Endpoint {
    /// The local node label.
    pub node: String,
    /// The source output or destination input port index.
    pub port: u16,
}

/// A single entry in the `(commits ...)` table.
#[derive(Clone, Debug)]
pub struct CommitDecl {
    /// This commit's own id, a concrete address or a file-local label.
    pub id: Addr,
    /// Seconds since the Unix epoch.
    pub secs: u64,
    /// Sub-second nanoseconds.
    pub nanos: u32,
    /// The parent commit, or `None` for a root commit.
    pub parent: Option<Addr>,
    /// Extra parents, the merged-in tips. Present only on merge commits.
    /// Written as a `(merge-parents ...)` clause only when non-empty.
    pub merge_parents: Vec<Addr>,
    /// The id of the graph this commit points at.
    pub graph: Addr,
}

/// A single entry in the `(names ...)` table.
#[derive(Clone, Debug)]
pub struct NameDecl {
    /// The registry name, a branch.
    pub name: String,
    /// The commit it points at.
    pub commit: Addr,
}

/// A generic metadata section form. Its shape is
/// `(section "<id>" (policy <p>) (liveness <l>) (entry <key> <datum>) ...)`.
///
/// Carries a registry section with its merge policy and liveness rule as data.
/// This includes sections from domains the reading application does not know,
/// so unknown sections round-trip through text.
#[derive(Clone, Debug)]
pub struct SectionForm {
    /// The section id, for example `"laser.palette"`.
    pub id: String,
    /// The section's merge policy.
    pub policy: gantz_ca::MergePolicy,
    /// The section's liveness rule.
    pub liveness: gantz_ca::Liveness,
    /// The entries. Each is a key plus an inline datum value.
    pub entries: Vec<(SectionKey, Datum)>,
}

/// A section entry key in text form.
///
/// Address keys are full hex with no prefix resolution. Section entries are
/// advisory metadata, so a key whose subject was re-rooted goes dead and the
/// next prune drops it.
#[derive(Clone, Debug)]
pub enum SectionKey {
    /// Keyed by a registry name.
    Name(String),
    /// Keyed by a commit address (full hex).
    Commit(String),
    /// Keyed by a graph address (full hex).
    Graph(String),
    /// Keyed by an arbitrary content address (full hex).
    Addr(String),
}
