//! The abstract syntax for the `.gantz` text format.
//!
//! A [`File`] is the intermediate representation between the raw datum reader
//! ([`super::sexpr`]) and the content-addressed registry ([`super::lower`] /
//! [`super::raise`]). It mirrors the textual forms one-to-one without resolving
//! any content addresses, so it is cheap to construct from either direction.

use serde_json::Value;

/// A parsed `.gantz` document: a sequence of top-level forms.
#[derive(Clone, Debug, Default)]
pub struct File {
    /// Graph definitions, in source order.
    pub graphs: Vec<GraphDef>,
    /// Layout/view sections.
    pub layouts: Vec<Layout>,
    /// Commit histories.
    pub histories: Vec<History>,
    /// Demo associations.
    pub demos: Vec<Demo>,
}

/// A top-level graph definition.
#[derive(Clone, Debug)]
pub struct GraphDef {
    /// How the graph is labelled (a name, or a concrete content address).
    pub id: GraphId,
    /// The graph interior.
    pub body: GraphBody,
}

/// The label of a top-level graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphId {
    /// A registry name (branch); this graph is the name's head body.
    Name(String),
    /// A concrete content address (hex string) for a non-head body.
    Addr(String),
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
    /// Explicit node index, overriding the sequential default (used to
    /// reproduce `StableGraph` holes); `None` means "previous index + 1".
    pub index: Option<usize>,
    /// The node specification.
    pub spec: NodeSpec,
}

/// A node specification.
#[derive(Clone, Debug)]
pub enum NodeSpec {
    /// A self-contained typetag node as a serde object `{ "type", ... }`.
    ///
    /// Covers every node with no file-level references to resolve (`expr`,
    /// `branch`, `inlet`, `number`, the generic `node` form, etc.).
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
    /// Optional pinned address; `None` resolves to the name's head commit.
    pub addr: Option<Addr>,
    /// Whether the reference should track the latest commit.
    pub sync: bool,
}

/// An address token: a concrete content address or a file-local label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Addr {
    /// A concrete content address as a hex string (full or unambiguous prefix).
    Concrete(String),
    /// A file-local placeholder, resolved to a computed address on load.
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

/// View/layout state for a graph.
#[derive(Clone, Debug)]
pub struct Layout {
    /// The graph name this layout applies to.
    pub graph: String,
    /// Descent path into nested graphs, as local node labels (empty = top).
    pub path: Vec<String>,
    /// Node positions: `(label, x, y)`.
    pub positions: Vec<(String, f32, f32)>,
    /// Optional scene rect `[min_x, min_y, max_x, max_y]`.
    pub scene: Option<[f32; 4]>,
}

/// The commit history for a name.
#[derive(Clone, Debug)]
pub struct History {
    /// The graph name this history applies to.
    pub graph: String,
    /// The commits, oldest first.
    pub commits: Vec<CommitDecl>,
}

/// A single commit within a history.
#[derive(Clone, Debug)]
pub struct CommitDecl {
    /// This commit's own address.
    pub id: Addr,
    /// Seconds since the Unix epoch.
    pub secs: u64,
    /// Sub-second nanoseconds.
    pub nanos: u32,
    /// The parent commit, or `None` for a root commit (`none`).
    pub parent: Option<Addr>,
    /// The graph body address; `None` means the head body for this name.
    pub graph: Option<Addr>,
}

/// A demo association for a name's head commit.
#[derive(Clone, Debug)]
pub struct Demo {
    /// The graph name.
    pub graph: String,
    /// The associated demo name.
    pub demo: String,
}
