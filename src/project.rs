use super::{Deserialize, Fail, From, Serialize};
use crate::graph::{self, GraphNode};
use crate::node::{self, Node, SerdeNode};
use quote::ToTokens;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::{fs, io, ops};

/// A gantz **Project** represents the context in which the user composes their gantz graph
/// together at runtime.
///
/// The **Project** is responsible for managing the project directory - the directory in which the
/// project can be saved and loaded and in which the workspace is situated and maintained.
///
/// Each project either shares an existing cargo workspace or has their own associated cargo
/// workspace, which stores all locally created **node** crates.
pub struct Project {
    /// Configuration information for cargo.
    ///
    /// Cargo is used to manage the project's workspace and its crates.
    cargo_config: cargo::Config,
    /// The path to the project directory.
    ///
    /// E.g. `~/gantz/projects/foo/`.
    directory: PathBuf,
    /// All nodes that have been imported into the project ready for use.
    nodes: NodeCollection,
}

/// A wrapper around a `Project` that behaves exactly like a `Project` but removes the project
/// directory and all its contents on `Drop`.
///
/// It also provides an alternate constructor that allows for specifying only the project name
/// rather than the entire path. In this case, the temporary project will be created within the
/// directory returned by `std::env::temp_dir()`.
pub struct TempProject {
    // An `Option` is used so that we can ensure the `Project` is dropped before cleaning up the
    // directory. This will always be `Some` for the lifetime of the `TempProject` until drop.
    project: Option<Project>,
}

/// A unique identifier representing an imported node.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct NodeId(u64);

/// The BTreeMap used for storing imported nodes.
pub type NodeTree = BTreeMap<NodeId, NodeKind>;

/// Stores all nodes that have been created within or imported into the project.
#[derive(Default, Deserialize, Serialize)]
pub struct NodeCollection {
    map: NodeTree,
}

/// A graph composed of IDs into the `NodeCollection`.
pub type NodeIdGraph = graph::StableGraph<NodeId>;

pub type NodeIdGraphNode = GraphNode<NodeIdGraph>;

/// Whether the node is a **Core** node (has no other internal **Node** dependencies) or is a
/// **Graph** node, composed entirely of other gantz **Node**s.
#[derive(Deserialize, Serialize)]
pub enum NodeKind {
    Core(Box<SerdeNode>),
    Graph(ProjectGraph),
}

/// A gantz node graph useful within gantz `Project`s.
///
/// This can be thought of as a node that is a graph composed of other nodes.
#[derive(Deserialize, Serialize)]
pub struct ProjectGraph {
    pub graph: NodeIdGraphNode,
    pub package_id: cargo::core::PackageId,
}

// A **Node** type constructed as a reference to some other node.
enum NodeRef<'a> {
    Core(&'a Node),
    Graph(GraphNode<graph::StableGraph<NodeRef<'a>>>),
}

/// Errors that may occur while creating a node crate.
#[derive(Debug, Fail, From)]
pub enum OpenNodePackageError {
    #[fail(display = "an IO error occurred: {}", err)]
    Io {
        #[fail(cause)]
        err: io::Error,
    },
    #[fail(display = "a cargo error occurred: {}", err)]
    Cargo {
        #[fail(cause)]
        err: failure::Error,
    },
    #[fail(display = "failed to update the manifest toml: {}", err)]
    UpdateTomlFile {
        #[fail(cause)]
        err: UpdateTomlFileError,
    },
}

/// Errors that may occur while checking an existing workspace or creating a new one.
#[derive(Debug, Fail, From)]
pub enum CreateOrCheckWorkspaceError {
    #[fail(display = "an IO error occurred: {}", err)]
    Io {
        #[fail(cause)]
        err: io::Error,
    },
    #[fail(display = "cargo failed to open the workspace: {}", err)]
    Cargo {
        #[fail(cause)]
        err: failure::Error,
    },
}

/// Errors that may occur while checking an existing project directory or creating a new one.
#[derive(Debug, Fail, From)]
pub enum CreateOrCheckProjectDirectoryError {
    #[fail(display = "an IO error occurred: {}", err)]
    Io {
        #[fail(cause)]
        err: io::Error,
    },
    #[fail(display = "cargo failed to open the workspace: {}", err)]
    Workspace {
        #[fail(cause)]
        err: CreateOrCheckWorkspaceError,
    },
}

/// Errors that may occur while creating a new project.
#[derive(Debug, Fail, From)]
pub enum ProjectOpenError {
    #[fail(display = "an IO error occurred: {}", err)]
    Io {
        #[fail(cause)]
        err: io::Error,
    },
    #[fail(display = "failed to initialise cargo config: {}", err)]
    CargoConfig {
        #[fail(cause)]
        err: failure::Error,
    },
    #[fail(display = "failed to create or check existing project directory: {}", err)]
    Project {
        #[fail(cause)]
        err: CreateOrCheckProjectDirectoryError,
    },
    #[fail(display = "failed to add the graph node to the collection: {}", err)]
    AddGraphNodeToCollection {
        #[fail(cause)]
        err: AddGraphNodeToCollectionError,
    },
    #[fail(display = "failed to compile the root graph node: {}", err)]
    GraphNodeCompile {
        #[fail(cause)]
        err: GraphNodeCompileError,
    }
}

/// Errors that might occur when saving or loading JSON from a file.
#[derive(Debug, Fail, From)]
pub enum JsonFileError {
    #[fail(display = "an IO error occurred: {}", err)]
    Io {
        #[fail(cause)]
        err: io::Error,
    },
    #[fail(display = "a JSON error occurred: {}", err)]
    Json {
        #[fail(cause)]
        err: serde_json::Error,
    },
}

/// Errors that may occur while creating a node crate.
#[derive(Debug, Fail, From)]
pub enum GraphNodeReplaceSrcError {
    #[fail(display = "an IO error occurred: {}", err)]
    Io {
        #[fail(cause)]
        err: io::Error,
    },
    #[fail(display = "a cargo error occurred: {}", err)]
    Cargo {
        #[fail(cause)]
        err: failure::Error,
    },
    #[fail(display = "no matching `package_id` in workspace that matches graph node `package_id`")]
    NoMatchingPackageId,
}

/// Errors that may occur while adding a graph node to a project's **NodeCollection**.
#[derive(Debug, Fail, From)]
pub enum AddGraphNodeToCollectionError {
    #[fail(display = "failed to open node cargo package: {}", err)]
    OpenNodePackage {
        #[fail(cause)]
        err: OpenNodePackageError,
    },
    #[fail(display = "failed to update the src/lib.rs of the graph node: {}", err)]
    GraphNodeReplaceSrc {
        #[fail(cause)]
        err: GraphNodeReplaceSrcError,
    },
}

/// Errors that might occur while updating the contents of a toml file.
#[derive(Debug, Fail, From)]
pub enum UpdateTomlFileError {
    #[fail(display = "an IO error occurred: {}", err)]
    Io {
        #[fail(cause)]
        err: io::Error,
    },
    #[fail(display = "failed to deserialize the toml: {}", err)]
    TomlDeserialize {
        #[fail(cause)]
        err: toml::de::Error,
    },
    #[fail(display = "failed to serialize the toml: {}", err)]
    TomlSerialize {
        #[fail(cause)]
        err: toml::ser::Error,
    }
}

/// Errors that might occur while compiling the project workspace.
#[derive(Debug, Fail, From)]
pub enum WorkspaceCompileError {
    #[fail(display = "a cargo error occurred: {}", err)]
    Cargo {
        #[fail(cause)]
        err: failure::Error,
    },
}

/// Errors that might occur while compiling a graph node.
#[derive(Debug, Fail, From)]
pub enum GraphNodeCompileError {
    #[fail(display = "a cargo error occurred: {}", err)]
    Cargo {
        #[fail(cause)]
        err: failure::Error,
    },
    #[fail(display = "no matching `package_id` in workspace that matches graph node `package_id`")]
    NoMatchingPackageId,
}

/// Errors that might occur while updating a `ProjectGraph`'s graph.
#[derive(Debug, Fail, From)]
pub enum UpdateGraphError {
    #[fail(display = "failed to replace graph node src: {}", err)]
    GraphNodeReplaceSrc {
        #[fail(cause)]
        err: GraphNodeReplaceSrcError,
    },
    #[fail(display = "failed to compile graph node: {}", err)]
    GraphNodeCompile {
        #[fail(cause)]
        err: GraphNodeCompileError,
    },
}

/// Node crates within the project workspace are prefixed with this.
pub const NODE_CRATE_PREFIX: &'static str = "gantz_node_";

impl Project {
    /// Open a project at the given directory path.
    ///
    /// If the project does not yet exist, it will be created.
    ///
    /// First, the project directory is prepared:
    ///
    /// - Creates the given project directory if it does not yet exist.
    /// - Creates the cargo workspace at `<proj_dir>/workspace` if it does not yet exist.
    /// - Initialises `<proj_dir>/workspace/Cargo.toml` with an empty members list if it does not
    ///   yet exist.
    ///
    /// The "name" of the project will match the final segment in the given directory path.
    ///
    /// Next, a crate for the project's root node is created:
    ///
    /// - Initialises a root node lib crate with the same name as the project. E.g.
    ///   `<proj_dir>/workspace/<proj_name>/`
    /// - Initialises `<proj_dir>/workspace/<proj_name>/Cargo.toml`.
    /// - Initialises an empty `<proj_dir>/workspace/<proj_name>/src/lib.rs` file.
    pub fn open(directory: PathBuf) -> Result<Self, ProjectOpenError> {
        let cargo_config = cargo::Config::default()?;

        // Prepare the project directory.
        create_or_check_project_dir(&directory, &cargo_config)?;

        // Load the collection of nodes.
        let node_collection_json_path = node_collection_json_path(&directory);
        let nodes = match NodeCollection::load(node_collection_json_path) {
            // TODO: Verify the node collection (e.g. `PackageId`s are correct, root node is a
            // graph with the same name as project).
            Ok(nodes) => nodes,
            // If no existing collection exists, create the default one.
            // TODO: Decipher between JSON errors and IO related errors for missing file.
            Err(_err) => {
                let mut nodes = NodeCollection::default();
                let graph = NodeIdGraph::default();
                let inlets = vec![];
                let outlets = vec![];
                let graph_node = GraphNode { graph, inlets, outlets };
                let ws_dir = workspace_dir(&directory);
                let proj_name = project_name(&directory);
                let node_id =
                    add_graph_node_to_collection(&ws_dir, proj_name, &cargo_config, graph_node, &mut nodes)?;
                if let Some(NodeKind::Graph(ref node)) = nodes.get(&node_id) {
                    graph_node_compile(&ws_dir, &cargo_config, node)?;
                }
                nodes
            }
        };

        let project = Project {
            cargo_config,
            directory,
            nodes,
        };
        Ok(project)
    }

    /// Add the given core node to the collection and return its unique identifier.
    pub fn add_core_node(&mut self, node: Box<SerdeNode>) -> NodeId {
        let kind = NodeKind::Core(node);
        let node_id = self.nodes.insert(kind);
        node_id
    }

    /// Add the given node to the collection and return its unique identifier.
    pub fn add_graph_node(
        &mut self,
        graph: NodeIdGraphNode,
        node_name: &str,
    ) -> Result<NodeId, AddGraphNodeToCollectionError> {
        let ws_dir = self.workspace_dir();
        let Project { ref cargo_config, ref mut nodes, .. } = *self;
        let n_id = add_graph_node_to_collection(ws_dir, node_name, cargo_config, graph, nodes)?;
        Ok(n_id)
    }

    /// Read-only access to the project's **NodeCollection**.
    pub fn nodes(&self) -> &NodeCollection {
        &self.nodes
    }

    /// The **NodeId** for the root graph node.
    pub fn root_node_id(&self) -> NodeId {
        NodeId(0)
    }

    /// The core node at the given **NodeId**.
    ///
    /// Returns `None` if there are no nodes for the given **NodeId** or if a node exists but it is
    /// not a **Core** node.
    pub fn core_node(&self, id: &NodeId) -> Option<&Box<SerdeNode>> {
        self.nodes.get(id).and_then(|kind| match kind {
            NodeKind::Core(ref node) => Some(node),
            _ => None,
        })
    }

    /// The graph node at the given **NodeId**.
    ///
    /// Returns `None` if there are no nodes for the given **NodeId** or if a node exists but it is
    /// not a **Graph** node.
    pub fn graph_node(&self, id: &NodeId) -> Option<&ProjectGraph> {
        self.nodes.get(id).and_then(|kind| match kind {
            NodeKind::Graph(ref graph) => Some(graph),
            _ => None,
        })
    }

    /// Update the graph associated with the graph node at the given **NodeId**.
    pub fn update_graph<F>(&mut self, id: &NodeId, update: F) -> Result<(), UpdateGraphError>
    where
        F: FnOnce(&mut NodeIdGraphNode),
    {
        match self.nodes.map.get_mut(id) {
            Some(NodeKind::Graph(ref mut node)) => update(&mut node.graph),
            _ => return Ok(()),
        }
        let file = graph_node_src(id, &self.nodes).expect("no graph node for NodeId");
        let ws_dir = self.workspace_dir();
        graph_node_replace_src(&ws_dir, &self.cargo_config, id, &self.nodes, file)?;
        let node = self.graph_node(id).expect("no graph node for NodeId");
        let _compilation = graph_node_compile(&ws_dir, &self.cargo_config, &node)?;
        Ok(())
    }

    /// The path to the generated dynamic library for the graph node at the given `id`.
    ///
    /// Returns `None` if there is no dynamic library or no graph node for the given `id`.
    pub fn graph_node_dylib(&self, id: &NodeId) -> cargo::CargoResult<Option<PathBuf>> {
        let node = match self.graph_node(id) {
            None => return Ok(None),
            Some(n) => n,
        };
        let ws_dir = self.workspace_dir();
        let ws_manifest_path = manifest_path(&ws_dir);
        let ws = cargo::core::Workspace::new(&ws_manifest_path, &self.cargo_config)?;
        let target_dir = ws.target_dir().into_path_unlocked();
        let pkg = match ws.members().find(|pkg| pkg.package_id() == node.package_id) {
            Some(pkg) => pkg,
            None => return Ok(None),
        };
        let target = match pkg.targets().iter().find(|target| target.is_dylib()) {
            Some(t) => t,
            None => return Ok(None),
        };
        let target_filestem = format!("lib{}", target.name());
        let target_path = target_dir
            .join("release")
            .join(target_filestem)
            .with_extension(dylib_ext());
        if target_path.exists() {
            Ok(Some(target_path))
        } else {
            Ok(None)
        }
    }

    /// The project directory.
    pub fn dir(&self) -> &Path {
        &self.directory
    }

    /// The path to the project's cargo workspace.
    pub fn workspace_dir(&self) -> PathBuf {
        workspace_dir(self.dir())
    }

    /// The project name.
    pub fn name(&self) -> &str {
        project_name(self.dir())
    }
}

impl TempProject {
    /// Create a new temporary project with the given project name.
    ///
    /// The project will be created within the directory returned by `std::env::temp_dir()` and
    /// will be removed when the `TempProject` is `Drop`ped.
    pub fn open_with_name(name: &str) -> Result<Self, ProjectOpenError> {
        let proj_dir = std::env::temp_dir().join(name);
        Self::open(proj_dir)
    }

    /// The same as `Project::open` but creates a temporary project.
    pub fn open(directory: PathBuf) -> Result<Self, ProjectOpenError> {
        let project = Some(Project::open(directory)?);
        Ok(TempProject { project })
    }
}

impl NodeCollection {
    // Load a node collection from the given path.
    fn load<P>(path: P) -> Result<Self, JsonFileError>
    where
        P: AsRef<Path>,
    {
        let file = fs::File::open(path)?;
        let t = serde_json::from_reader(file)?;
        Ok(t)
    }

    // The next unique identifier that will be produced for the next node to be inserted into the
    // collection.
    fn next_node_id(&self) -> NodeId {
        self.keys()
            .last()
            .map(|&NodeId(u)| NodeId(u.checked_add(1).expect("no unique `NodeId`s remaining")))
            .unwrap_or(NodeId(0))
    }

    // Insert the given node and return the unique `NodeId` key associated with it.
    fn insert(&mut self, node: NodeKind) -> NodeId {
        let id = self.next_node_id();
        self.map.insert(id, node);
        id
    }
}

impl<'a> Node for NodeRef<'a> {
    fn evaluator(&self) -> node::Evaluator {
        match self {
            NodeRef::Core(node) => node.evaluator(),
            NodeRef::Graph(graph) => graph.evaluator(),
        }
    }

    fn push_eval(&self) -> Option<node::PushEval> {
        match self {
            NodeRef::Core(node) => node.push_eval(),
            NodeRef::Graph(graph) => graph.push_eval(),
        }
    }

    fn pull_eval(&self) -> Option<node::PullEval> {
        match self {
            NodeRef::Core(node) => node.pull_eval(),
            NodeRef::Graph(graph) => graph.pull_eval(),
        }
    }
}

impl ops::Deref for TempProject {
    type Target = Project;
    fn deref(&self) -> &Self::Target {
        self.project.as_ref().expect("inner `project` was `None`")
    }
}

impl ops::DerefMut for TempProject {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.project.as_mut().expect("inner `project` was `None`")
    }
}

impl ops::Deref for NodeCollection {
    type Target = NodeTree;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let dir = self.dir().to_path_buf();
        std::mem::drop(self.project.take());
        std::fs::remove_dir_all(dir).ok();
    }
}

// Get the dylib extension for this platform.
//
// TODO: This should be exposed from cargo.
fn dylib_ext() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        return "so";
    }
    #[cfg(target_os = "macos")]
    {
        return "dylib";
    }
    #[cfg(target_os = "windows")]
    {
        return "dll";
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        panic!("unknown dynamic library for this platform")
    }
}

/// Given the project directory, retrieve the project name from the file stem.
pub fn project_name(project_dir: &Path) -> &str {
    project_dir
        .file_stem()
        .expect("failed to retrieve `file_stem` from project directory path")
        .to_str()
        .expect("failed to parse project `file_stem` as valid UTF-8")
}

/// Given the project directory, return the path to the project's workspace directory.
pub fn workspace_dir<P>(project_dir: P) -> PathBuf
where
    P: AsRef<Path>,
{
    project_dir.as_ref().join("workspace")
}

/// Given a crate directory, return a path to its `Cargo.toml` file.
pub fn manifest_path<P>(crate_dir: P) -> PathBuf
where
    P: AsRef<Path>,
{
    crate_dir.as_ref().join("Cargo.toml")
}

/// The path at which the project's node collection JSON is stored.
pub fn node_collection_json_path<P>(project_dir: P) -> PathBuf
where
    P: AsRef<Path>,
{
    project_dir.as_ref().join("node_collection.json")
}

/// Given some UTF-8 node name, return the name of the crate.
pub fn node_crate_name(node_name: &str) -> String {
    format!("{}{}", NODE_CRATE_PREFIX, slug::slugify(node_name).replace("-", "_"))
}

/// Given the workspace directory and some UTF-8 node name, return the path to the crate directory.
pub fn node_crate_dir<P>(workspace_dir: P, node_name: &str) -> PathBuf
where
    P: AsRef<Path>,
{
    workspace_dir.as_ref().join(node_crate_name(node_name))
}

/// Given a node crate directory, return the path to the src directory.
pub fn node_crate_src<P>(node_crate_dir: P) -> PathBuf
where
    P: AsRef<Path>,
{
    node_crate_dir.as_ref().join("src")
}

/// Given a node crate directory, return the path to the lib.rs file.
pub fn node_crate_lib_rs<P>(node_crate_src: P) -> PathBuf
where
    P: AsRef<Path>,
{
    node_crate_src.as_ref().join("lib.rs")
}

// Check the project at the given directory or create it if it does not exist.
//
// This does the following steps:
//
// - Creates the given project directory if it does not yet exist.
// - Creates the cargo workspace at `<path>/workspace` if it does not yet exist.
// - Initialises `<path>/workspace/Cargo.toml` with an empty members list if it does not yet exist.
fn create_or_check_project_dir<P>(
    project_dir: P,
    cargo_config: &cargo::Config,
) -> Result<(), CreateOrCheckProjectDirectoryError>
where
    P: AsRef<Path>,
{
    // Create the project directory.
    let project_dir = project_dir.as_ref();
    if !project_dir.exists() {
        fs::create_dir_all(project_dir)?;
    }

    // Open the existing workspace or create it if it does not exist.
    let workspace_dir = workspace_dir(project_dir);
    create_or_check_workspace(workspace_dir, cargo_config)?;

    Ok(())
}

// If a workspace does not exist at the given directory, create one.
//
// Returns an error if some IO error occurs of if cargo does not consider the existing/created
// workspace to be a valid one.
fn create_or_check_workspace<P>(
    workspace_dir: P,
    cargo_config: &cargo::Config,
) -> Result<(), CreateOrCheckWorkspaceError>
where
    P: AsRef<Path>,
{
    // Create the workspace directory.
    let workspace_dir = workspace_dir.as_ref();
    if !workspace_dir.exists() {
        fs::create_dir(&workspace_dir)?;
    }

    // Create the workspace cargo toml.
    let workspace_manifest_path = manifest_path(&workspace_dir);
    create_workspace_cargo_toml(&workspace_manifest_path)?;

    // Verify the workspace.
    cargo::core::Workspace::new(&workspace_manifest_path, cargo_config)?;

    Ok(())
}

// Create a workspace `Cargo.toml` file at the given path if it does not yet exist.
fn create_workspace_cargo_toml<P>(toml_path: P) -> io::Result<()>
where
    P: AsRef<Path>,
{
    // If the file already exists, don't do anything.
    let toml_path = toml_path.as_ref();
    if toml_path.exists() {
        return Ok(());
    }

    // Create the toml to write to the file.
    let toml_str = "[workspace]\nmembers = [\n]";

    // Write the string to a file at the given path.
    fs::write(toml_path, toml_str)
}

// Update the toml at the given file.
fn update_toml_file<P, F>(toml_path: P, update: F) -> Result<(), UpdateTomlFileError>
where
    P: AsRef<Path>,
    F: FnOnce(&mut toml::Value),
{
    let bytes = fs::read(&toml_path)?;
    let mut toml: toml::Value = toml::from_slice(&bytes)?;
    update(&mut toml);
    let toml_string = toml::to_string_pretty(&toml)?;
    fs::write(&toml_path, &toml_string)?;
    Ok(())
}

fn node_crate_new_options(dir_path: PathBuf) -> cargo::CargoResult<cargo::ops::NewOptions> {
    let version_ctrl = None;
    let bin = false;
    let lib = true;
    let name = None;
    let edition = None;
    let registry = None;
    cargo::ops::NewOptions::new(version_ctrl, bin, lib, dir_path, name, edition, registry)
}

// Create a node crate within the given workspace directory with the given name.
//
// The crate name will be slugified before being used within the path.
fn open_node_package<P>(
    workspace_dir: P,
    node_name: &str,
    cargo_config: &cargo::Config,
) -> Result<cargo::core::PackageId, OpenNodePackageError>
where
    P: AsRef<Path>,
{
    // Check to see if the node exists within `workspace.members` yet. If not, add it.
    let workspace_dir = workspace_dir.as_ref();
    let node_crate_name = node_crate_name(node_name);
    let workspace_manifest_path = manifest_path(workspace_dir);
    let exists = {
        let workspace = cargo::core::Workspace::new(&workspace_manifest_path, &cargo_config)?;
        workspace.members().any(|pkg| format!("{}", pkg.name()) == node_crate_name)
    };
    if !exists {
        update_toml_file(&workspace_manifest_path, |toml| {
            let table = match toml {
                toml::Value::Table(ref mut table) => table,
                _ => return,
            };
            let ws = match table.get_mut("workspace") {
                Some(toml::Value::Table(ref mut ws)) => ws,
                _ => return,
            };
            if let Some(toml::Value::Array(ref mut members)) = ws.get_mut("members") {
                members.push(node_crate_name.clone().into());
            }
        })?;
    }

    // If the directory doesn't exist yet, create it.
    let node_crate_dir_path = node_crate_dir(workspace_dir, node_name);
    if !node_crate_dir_path.exists() {
        let new_options = node_crate_new_options(node_crate_dir_path.clone())?;
        cargo::ops::new(&new_options, &cargo_config)?;

        // Add the lib targets.
        let node_crate_manifest_path = manifest_path(&node_crate_dir_path);
        update_toml_file(&node_crate_manifest_path, |toml| {
            let table = match toml {
                toml::Value::Table(ref mut table) => table,
                _ => return,
            };
            let mut lib_table = HashMap::default();
            lib_table.insert("name".to_string(), node_crate_name.clone().into());
            let array = toml::Value::Array(vec!["lib".into(), "dylib".into()]);
            lib_table.insert("crate-type".to_string(), array);
            table.insert("lib".to_string(), lib_table.into());
        })?;
    }

    // Verify the package after creation (or if it already exists) by reading it.
    let workspace = cargo::core::Workspace::new(&workspace_manifest_path, &cargo_config)?;
    let pkg = workspace
        .members()
        .find(|pkg| format!("{}", pkg.name()) == node_crate_name)
        .expect("failed to find workspace package with matching name");

    Ok(pkg.package_id())
}

// Add the given node to the node collection and return the unique `NodeId` and generated
// cargo workspace package associated with it.
fn add_graph_node_to_collection<P>(
    workspace_dir: P,
    node_name: &str,
    cargo_config: &cargo::Config,
    graph: NodeIdGraphNode,
    nodes: &mut NodeCollection,
) -> Result<NodeId, AddGraphNodeToCollectionError>
where
    P: AsRef<Path>,
{
    let package_id = open_node_package(&workspace_dir, node_name, cargo_config)?;
    let kind = NodeKind::Graph(ProjectGraph { graph, package_id });
    let node_id = nodes.insert(kind);
    let file = graph_node_src(&node_id, nodes).expect("no graph node for NodeId");
    graph_node_replace_src(&workspace_dir, cargo_config, &node_id, nodes, file)?;
    Ok(node_id)
}

// Compile all crates within the workspace.
fn workspace_compile<P>(
    workspace_dir: P,
    cargo_config: &cargo::Config,
) -> Result<HashMap<cargo::core::PackageId, cargo::core::compiler::Compilation>, WorkspaceCompileError>
where
    P: AsRef<Path>,
{
    let ws_manifest_path = manifest_path(workspace_dir);
    let ws = cargo::core::Workspace::new(&ws_manifest_path, &cargo_config)?;
    let mut compilations = HashMap::default();
    for pkg in ws.members() {
        let pkg_manifest_path = pkg.manifest_path();
        let pkg_ws = cargo::core::Workspace::new(&pkg_manifest_path, &cargo_config)?;
        let mode = cargo::core::compiler::CompileMode::Build;
        let mut options = cargo::ops::CompileOptions::new(&cargo_config, mode)?;
        options.build_config.message_format = cargo::core::compiler::MessageFormat::Json;
        options.build_config.release = true;
        let compilation = cargo::ops::compile(&pkg_ws, &options)?;
        compilations.insert(pkg.package_id(), compilation);
    }
    Ok(compilations)
}

// Compile the graph node associated with the given `NodeId`.
fn graph_node_compile<'conf, P>(
    workspace_dir: P,
    cargo_config: &'conf cargo::Config,
    node: &ProjectGraph,
) -> Result<cargo::core::compiler::Compilation<'conf>, GraphNodeCompileError>
where
    P: AsRef<Path>,
{
    let ws_manifest_path = manifest_path(workspace_dir);
    let ws = cargo::core::Workspace::new(&ws_manifest_path, &cargo_config)?;
    let pkg = ws.members()
        .find(|pkg| pkg.package_id() == node.package_id)
        .ok_or(GraphNodeCompileError::NoMatchingPackageId)?;
    let pkg_manifest_path = pkg.manifest_path();
    let pkg_ws = cargo::core::Workspace::new(&pkg_manifest_path, &cargo_config)?;
    let mode = cargo::core::compiler::CompileMode::Build;
    let mut options = cargo::ops::CompileOptions::new(&cargo_config, mode)?;
    options.build_config.message_format = cargo::core::compiler::MessageFormat::Json;
    options.build_config.release = true;
    let compilation = cargo::ops::compile(&pkg_ws, &options)?;
    Ok(compilation)
}

// Given a `NodeIdGraphNode` and `NodeCollection`, return a graph capable of evaluation.
fn id_graph_to_node_graph<'a>(
    g: &NodeIdGraphNode,
    ns: &'a NodeCollection,
) -> GraphNode<graph::StableGraph<NodeRef<'a>>> {
    let inlets = g.inlets.clone();
    let outlets = g.outlets.clone();
    let graph = g.graph.map(
        |_, n_id| {
            match ns[n_id] {
                NodeKind::Core(ref node) => NodeRef::Core(node.node()),
                NodeKind::Graph(ref node) => {
                    NodeRef::Graph(id_graph_to_node_graph(&node.graph, ns))
                }
            }
        },
        |_, edge| {
            edge.clone()
        },
    );
    GraphNode { graph, inlets, outlets }
}

// Generate a src file for the graph node associated with the given `NodeId`.
//
// Returns `None` if there is no graph node associated with the given `NodeId`.
fn graph_node_src(id: &NodeId, nodes: &NodeCollection) -> Option<syn::File> {
    if let Some(NodeKind::Graph(ref node)) = nodes.get(id) {
        let graph = id_graph_to_node_graph(&node.graph, nodes);
        return Some(graph::codegen::file(&graph.graph, &graph.inlets, &graph.outlets));
    }
    None
}

// Replace the `src/lib.rs` file for the given graph node with the given file. For use in
// conjunction with `graph_node_src`.
fn graph_node_replace_src<P>(
    workspace_dir: P,
    cargo_config: &cargo::Config,
    id: &NodeId,
    nodes: &NodeCollection,
    file: syn::File,
) -> Result<(), GraphNodeReplaceSrcError>
where
    P: AsRef<Path>,
{
    if let Some(NodeKind::Graph(ref node)) = nodes.get(id) {
        let ws_manifest_path = manifest_path(workspace_dir);
        let workspace = cargo::core::Workspace::new(&ws_manifest_path, &cargo_config)?;
        let pkg = workspace
            .members()
            .find(|pkg| pkg.package_id() == node.package_id)
            .ok_or(GraphNodeReplaceSrcError::NoMatchingPackageId)?;
        let node_crate_dir = pkg.root();
        let node_crate_lib_rs = node_crate_lib_rs(node_crate_src(node_crate_dir));
        let src_string = format!("{}", file.into_token_stream());
        let src_bytes = src_string.as_bytes();
        std::fs::write(&node_crate_lib_rs, src_bytes)?;
    }
    Ok(())
}
