use super::{Deserialize, Serialize};
use crate::node::{self, Node};
use std::str::FromStr;

/// A wrapper around a `Node` that adds a set of crate dependencies.
///
/// The implementation of `Node` will match the inner node type `N`, but with a unique
/// implementation of `Node::crate_deps` that returns the combination of the inner node's crate
/// dependencies and the specified list crate dependencies.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Deps<N> {
    node: N,
    crate_deps: Vec<node::CrateDep>,
}

/// A trait implemented for all `Node` types allowing to easily add crate dependencies.
pub trait WithCrateDeps: Sized + Node {
    /// Consume `self` and return a `Node` with the given crate depenendencies.
    fn with_crate_deps(self, deps: Vec<node::CrateDep>) -> Deps<Self>;

    /// The same as `with_crate_deps` but allows for dependencies via parsable strings.
    fn with_deps<I>(self, deps: I) -> Result<Deps<Self>, node::ParseCrateDepError>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let mut crate_deps = vec![];
        for d in deps {
            crate_deps.push(node::CrateDep::from_str(d.as_ref())?);
        }
        Ok(self.with_crate_deps(crate_deps))
    }

    /// The same as `with_deps`, but for specifying only a single dependency.
    fn with_dep(self, dep: &str) -> Result<Deps<Self>, node::ParseCrateDepError> {
        self.with_deps(Some(dep))
    }
}

impl<N> Deps<N>
where
    N: Node,
{
    /// Given some node, return a `Deps` node with the given crate depenedencies.
    pub fn new(node: N, crate_deps: Vec<node::CrateDep>) -> Self {
        Deps { node, crate_deps }
    }
}

impl<N> WithCrateDeps for N
where
    N: Node,
{
    fn with_crate_deps(self, deps: Vec<node::CrateDep>) -> Deps<Self> {
        Deps::new(self, deps)
    }
}

impl<N> Node for Deps<N>
where
    N: Node,
{
    fn evaluator(&self) -> node::Evaluator {
        self.node.evaluator()
    }

    fn push_eval(&self) -> Option<node::EvalFn> {
        self.node.push_eval()
    }

    fn pull_eval(&self) -> Option<node::EvalFn> {
        self.node.pull_eval()
    }

    fn state_type(&self) -> Option<syn::Type> {
        self.node.state_type()
    }

    fn crate_deps(&self) -> Vec<node::CrateDep> {
        self.crate_deps.clone()
    }
}
