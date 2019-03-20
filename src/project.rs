use crate::graph;
use crate::node::Node;
use std::{fs, io, ops};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

/// A unique identifier representing an imported node.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct NodeId(u64);

/// The BTreeMap used for storing imported nodes.
pub type NodeTree = BTreeMap<NodeId, NodeContainer>;

/// Stores all nodes that have been created within or imported into the project.
#[derive(Default, Deserialize, Serialize)]
pub struct NodeCollection {
    map: NodeTree,
}

/// A node, either derived from rust code or composed from a graph of other nodes.
#[derive(Deserialize, Serialize)]
pub struct NodeContainer {
    kind: NodeKind,
    package_id: cargo::core::PackageId,
}

/// A graph composed of IDs into the `NodeCollection`.
pub type NodeIdGraph = graph::Petgraph<NodeId>;

/// Whether the node is an endpoint (has no more `Node` dependencies) or is a `Graph` composed of
/// other `Node`s.
#[derive(Deserialize, Serialize)]
pub enum NodeKind {
    Endpoint(Box<Node>),
    Graph(NodeIdGraph),
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
    #[fail(display = "failed to deserialize the manifest toml: {}", err)]
    ManifestDeserialize {
        #[fail(cause)]
        err: toml::de::Error,
    },
    #[fail(display = "failed to serialize the manifest toml: {}", err)]
    ManifestSerialize {
        #[fail(cause)]
        err: toml::ser::Error,
    }
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
    #[fail(display = "failed to create or check existing crate for project root node: {}", err)]
    NodeCrate {
        #[fail(cause)]
        err: OpenNodePackageError,
    },
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

/// Node crates within the project workspace are prefixed with this.
pub const NODE_CRATE_PREFIX: &'static str = "gantz-node-";

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
                let kind = NodeKind::Graph(graph);
                let workspace_dir = workspace_dir(&directory);
                let proj_name = project_name(&directory);
                add_node_to_collection(workspace_dir, proj_name, &cargo_config, kind, &mut nodes)?;
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

    /// Add the given node to the collection and return it's unique identifier.
    pub fn add_node(
        &mut self,
        kind: NodeKind,
        node_name: &str,
    ) -> Result<NodeId, OpenNodePackageError> {
        let ws_dir = self.workspace_dir();
        let Project { ref cargo_config, ref mut nodes, .. } = *self;
        let (n_id, _) = add_node_to_collection(ws_dir, node_name, cargo_config, kind, nodes)?;
        Ok(n_id)
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
    fn insert(&mut self, node: NodeContainer) -> NodeId {
        let id = self.next_node_id();
        self.map.insert(id, node);
        id
    }
}

impl ops::Deref for NodeCollection {
    type Target = NodeTree;
    fn deref(&self) -> &Self::Target {
        &self.map
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

/// Given a project's workspace directory, return a path to its `Cargo.toml` file.
pub fn workspace_manifest_path<P>(workspace_dir: P) -> PathBuf
where
    P: AsRef<Path>,
{
    workspace_dir.as_ref().join("Cargo.toml")
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
    format!("{}{}", NODE_CRATE_PREFIX, slug::slugify(node_name))
}

/// Given the workspace directory and some UTF-8 node name, return the path to the crate directory.
pub fn node_crate_dir<P>(workspace_dir: P, node_name: &str) -> PathBuf
where
    P: AsRef<Path>,
{
    workspace_dir.as_ref().join(node_crate_name(node_name))
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
    let workspace_manifest_path = workspace_manifest_path(&workspace_dir);
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

// Create a node crate within the given workspace directory with the given name.
//
// The crate name will be slugified before being used within the path.
fn open_node_package<P>(
    workspace_dir: P,
    node_name: &str,
    cargo_config: &cargo::Config,
) -> Result<cargo::core::Package, OpenNodePackageError>
where
    P: AsRef<Path>,
{
    // Check to see if the node exists within `workspace.members` yet. If not, add it.
    let workspace_dir = workspace_dir.as_ref();
    let node_crate_name = node_crate_name(node_name);
    let workspace_manifest_path = workspace_manifest_path(workspace_dir);
    let exists = {
        let workspace = cargo::core::Workspace::new(&workspace_manifest_path, &cargo_config)?;
        workspace.members().any(|pkg| format!("{}", pkg.name()) == node_crate_name)
    };
    if !exists {
        let bytes = fs::read(&workspace_manifest_path)?;
        let mut toml: toml::Value = toml::from_slice(&bytes)?;
        if let toml::Value::Table(ref mut table) = toml {
            if let Some(toml::Value::Table(ref mut workspace)) = table.get_mut("workspace") {
                if let Some(toml::Value::Array(ref mut members)) = workspace.get_mut("members") {
                    members.push(node_crate_name.into());
                }
            }
        }
        let toml_string = toml::to_string_pretty(&toml)?;
        fs::write(workspace_manifest_path, &toml_string)?;
    }

    // If the directory doesn't exist yet, create it.
    let node_crate_dir_path = node_crate_dir(workspace_dir, node_name);
    if !node_crate_dir_path.exists() {
        let version_ctrl = None;
        let bin = false;
        let lib = true;
        let name = None;
        let edition = None;
        let registry = None;
        let new_options = cargo::ops::NewOptions::new(
            version_ctrl,
            bin,
            lib,
            node_crate_dir_path.clone(),
            name,
            edition,
            registry,
        )?;
        cargo::ops::new(&new_options, &cargo_config)?;
    }

    // Verify the package after creation (or if it already exists) by reading it.
    let manifest_path = node_crate_dir_path.join("Cargo.toml");
    let src_id = cargo::core::SourceId::crates_io(&cargo_config)?;
    let (pkg, _nested) = cargo::ops::read_package(&manifest_path, src_id, &cargo_config)?;

    Ok(pkg)
}

// Add the given node to the node collection and return the unique `NodeId` and generated
// cargo workspace package associated with it.
fn add_node_to_collection<P>(
    workspace_dir: P,
    node_name: &str,
    cargo_config: &cargo::Config,
    kind: NodeKind,
    nodes: &mut NodeCollection,
) -> Result<(NodeId, cargo::core::Package), OpenNodePackageError>
where
    P: AsRef<Path>,
{
    let pkg = open_node_package(workspace_dir, node_name, cargo_config)?;
    let package_id = pkg.package_id();
    let container = NodeContainer { kind, package_id };
    let node_id = nodes.insert(container);
    Ok((node_id, pkg))
}
