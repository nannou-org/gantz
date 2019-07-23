//! **Gantz** is a programming execution representation.
//!
//! **Gantz** uses a directed graph for this representation. **Node**s represent expressions, while
//! the edges between nodes define the order of evaluation for each of these expressions.
//!
//! - **Inlet**s of a node describe the inputs to the expression.
//! - **Outlet**s of a node describe the outputs of the evaluated expression.
//!
//! **Gantz** allows for triggering evaluation of the graph in two ways:
//!
//! 1. **Push evaluation**. The graph allows for "pushing" evaluation from one or more outlets of a
//!    single node. This causes the "pushed" outlets to begin evaluation in visit-order of a
//!    breadth-first-search that ends when nodes are reached that either 1. only have outlets
//!    connecting to nodes that have already been evaluated or 2. have no outlets at all.
//!
//! 2. **Pull evaluation**. The graph allows for "pulling" evaluation from one or more inlets of a
//!    single node. This causes the "pulled" inlets to perform a depth-first search in order to
//!    find all connected nodes that either 1. Have no inlets or 2. have inlets that connect to
//!    already visited nodes. Once these "starting" nodes are found, evaluation is "pushed" from
//!    each of these nodes in the order in which they were visited.
//!
//! ## Current Questions

use derive_more::From;
use failure::Fail;
use serde::{self, Deserialize, Serialize};

pub mod project;

pub use gantz_core::{self as core, graph, node, Edge, Node};
pub use project::{Project, TempProject};
