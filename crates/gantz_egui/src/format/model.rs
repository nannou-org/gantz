//! The abstract syntax for the `.gantz` text format.
//!
//! A [`File`] is the intermediate representation between the reader
//! ([`super::parse`]) and the content-addressed registry ([`super::lower`] /
//! [`super::raise`]). It mirrors the registry's three maps: graph bodies, a
//! `(commits ...)` table and a `(names ...)` table, plus layout and demos.

use serde_json::Value;

/// A parsed `.gantz` document.
#[derive(Clone, Debug, Default)]
pub struct File {
    /// Graph bodies, in source order.
    pub graphs: Vec<GraphDef>,
    /// Layout/view sections, keyed by graph id.
    pub layouts: Vec<Layout>,
    /// The flat commit table (at most one head commit per graph).
    pub commits: Vec<CommitDecl>,
    /// Name -> commit mappings.
    pub names: Vec<NameDecl>,
    /// Demo associations.
    pub demos: Vec<Demo>,
}

/// A graph body, identified by a file-local id.
#[derive(Clone, Debug)]
pub struct GraphDef {
    /// The graph's file-local id: a concrete graph address (string) or a label
    /// (symbol). A label that no `(commits ...)` entry references is treated as
    /// a registry name with a synthesised root commit (the hand-authoring path).
    pub id: Addr,
    /// The graph interior.
    pub body: GraphBody,
}

/// The interior of a graph: node declarations (in index order) and connections.
#[derive(Clone, Debug, Default)]
pub struct GraphBody {
    /// Node declarations; declaration order is the node index by default.
    pub nodes: Vec<NodeDecl>,
    /// Connections between node ports.
    pub conns: Vec<Conn>,
}

/// A single node declaration within a graph.
#[derive(Clone, Debug)]
pub struct NodeDecl {
    /// File-local label, referenced by connections and layout.
    pub name: String,
    /// Explicit node index, overriding the sequential default (reserved for
    /// reproducing `StableGraph` holes); `None` means "previous index + 1".
    pub index: Option<usize>,
    /// The node specification.
    pub spec: NodeSpec,
}

/// A node specification.
#[derive(Clone, Debug)]
pub enum NodeSpec {
    /// A self-contained typetag node as a serde object `{ "type", ... }`.
    Value(Value),
    /// An inline nested graph, lowered to a `GraphNode`.
    Graph(GraphBody),
    /// A `NamedRef`/`FnNamedRef` whose address resolves at load time.
    Ref(RefSpec),
}

/// A reference to another graph by name.
#[derive(Clone, Debug)]
pub struct RefSpec {
    /// `true` for `fn-ref` (`FnNamedRef`), `false` for `ref` (`NamedRef`).
    pub func: bool,
    /// The referenced name.
    pub name: String,
    /// Optional pinned commit; `None` resolves to the name's head commit.
    pub addr: Option<Addr>,
    /// Whether the reference should track the latest commit.
    pub sync: bool,
}

/// A file-local address token: a concrete content address or a label.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Addr {
    /// A concrete content address as a hex string (full or unambiguous prefix).
    Concrete(String),
    /// A file-local label (symbol), resolved to a computed address on load.
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

/// One end of a connection: a node label and a port index.
#[derive(Clone, Debug)]
pub struct Endpoint {
    /// The local node label.
    pub node: String,
    /// The output (source) or input (destination) port index.
    pub port: u16,
}

/// View/layout state for a graph, keyed by the graph's id.
#[derive(Clone, Debug)]
pub struct Layout {
    /// The id of the graph this layout applies to.
    pub graph: Addr,
    /// Node positions: `(label, x, y)`.
    pub positions: Vec<(String, f32, f32)>,
    /// Optional scene rect `[min_x, min_y, max_x, max_y]`.
    pub scene: Option<[f32; 4]>,
}

/// A single entry in the `(commits ...)` table.
#[derive(Clone, Debug)]
pub struct CommitDecl {
    /// This commit's own id (a concrete address or a file-local label).
    pub id: Addr,
    /// Seconds since the Unix epoch.
    pub secs: u64,
    /// Sub-second nanoseconds.
    pub nanos: u32,
    /// The parent commit, or `None` for a root commit.
    pub parent: Option<Addr>,
    /// The id of the graph this commit points at.
    pub graph: Addr,
}

/// A single entry in the `(names ...)` table.
#[derive(Clone, Debug)]
pub struct NameDecl {
    /// The registry name (branch).
    pub name: String,
    /// The commit it points at.
    pub commit: Addr,
}

/// A demo association: a name and its demo string.
#[derive(Clone, Debug)]
pub struct Demo {
    /// The graph name.
    pub graph: String,
    /// The associated demo name.
    pub demo: String,
}
