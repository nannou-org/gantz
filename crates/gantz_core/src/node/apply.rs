//! A node that applies a function to a list of arguments.

use crate::node;
use gantz_ca::CaHash;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A fixed apply argument count outside the supported range was requested.
#[derive(Clone, Copy, Debug, Error, Eq, Hash, PartialEq, PartialOrd, Ord)]
#[error(
    "fixed apply arg count must be between {min} and {max}, got {0}",
    min = FixedArgCount::MIN,
    max = FixedArgCount::MAX
)]
pub struct InvalidFixedArgCount(pub usize);

/// A validated fixed argument count for [`Apply`] separate-arg mode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Serialize, CaHash)]
#[cahash("gantz.apply.fixed-arg-count")]
pub struct FixedArgCount(u8);

impl FixedArgCount {
    /// The minimum supported fixed argument count.
    pub const MIN: usize = 1;
    /// The maximum supported fixed argument count.
    pub const MAX: usize = 10;

    /// Get the count as a `usize`.
    pub fn get(self) -> usize {
        self.0 as usize
    }
}

impl TryFrom<usize> for FixedArgCount {
    type Error = InvalidFixedArgCount;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        if !(Self::MIN..=Self::MAX).contains(&value) {
            return Err(InvalidFixedArgCount(value));
        }
        Ok(Self(value as u8))
    }
}

impl<'de> Deserialize<'de> for FixedArgCount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let count = u8::deserialize(deserializer)?;
        Self::try_from(count as usize).map_err(serde::de::Error::custom)
    }
}

/// The mode by which an [`Apply`] node receives function arguments.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.apply.mode")]
pub enum ArgMode {
    /// A single input receives a list of arguments.
    #[default]
    List,
    /// A dedicated input is exposed for each argument.
    Fixed(FixedArgCount),
}

impl ArgMode {
    fn from_fixed_arg_count(fixed_arg_count: Option<usize>) -> Result<Self, InvalidFixedArgCount> {
        fixed_arg_count
            .map(FixedArgCount::try_from)
            .transpose()
            .map(|fixed_arg_count| fixed_arg_count.map_or(Self::List, Self::Fixed))
    }

    fn fixed_arg_count(self) -> Option<usize> {
        match self {
            Self::List => None,
            Self::Fixed(arg_count) => Some(arg_count.get()),
        }
    }

    fn node_input_count(self) -> usize {
        1 + self.arg_input_count()
    }

    fn arg_input_count(self) -> usize {
        self.fixed_arg_count().unwrap_or(1)
    }

    fn arg_input_indices(self) -> Vec<usize> {
        (1..=self.arg_input_count()).collect()
    }

    fn cleared_inputs_for_transition(self, next: Self) -> Vec<usize> {
        match (self, next) {
            (Self::List, Self::List) => Vec::new(),
            (Self::Fixed(current), Self::Fixed(next)) if next >= current => Vec::new(),
            (Self::Fixed(current), Self::Fixed(next)) => {
                ((next.get() + 1)..=current.get()).collect()
            }
            (Self::List, Self::Fixed(_)) | (Self::Fixed(_), Self::List) => self.arg_input_indices(),
        }
    }
}

/// A node that applies a function to arguments.
///
/// In other words, this node "calls" the function received on the first input
/// with the arguments received on the second input, or on a configurable set
/// of separate argument inputs.
///
/// The node is stateless and evaluates immediately when a function is received.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.apply")]
pub struct Apply {
    arg_mode: ArgMode,
}

impl Apply {
    /// The maximum number of separate argument inputs.
    pub const MAX_FIXED_ARGS: usize = FixedArgCount::MAX;

    /// Construct an [`Apply`] node using a single list input for arguments.
    pub fn list() -> Self {
        Self::default()
    }

    /// Construct an [`Apply`] node with one input per argument.
    pub fn fixed(arg_count: usize) -> Result<Self, InvalidFixedArgCount> {
        Ok(Self {
            arg_mode: ArgMode::Fixed(FixedArgCount::try_from(arg_count)?),
        })
    }

    /// The current argument mode.
    pub fn arg_mode(&self) -> ArgMode {
        self.arg_mode
    }

    /// The current fixed argument count, if separate-arg mode is enabled.
    pub fn fixed_arg_count(&self) -> Option<usize> {
        self.arg_mode.fixed_arg_count()
    }

    /// Return a reconfigured apply node and the arg-side inputs that should be
    /// cleared when transitioning to the given mode.
    pub fn reconfigured(
        &self,
        fixed_arg_count: Option<usize>,
    ) -> Result<(Self, Vec<usize>), InvalidFixedArgCount> {
        let arg_mode = ArgMode::from_fixed_arg_count(fixed_arg_count)?;
        let next = Self { arg_mode };
        let cleared_inputs = self.arg_mode.cleared_inputs_for_transition(arg_mode);
        Ok((next, cleared_inputs))
    }

    fn input_expr(inputs: &[Option<String>], index: usize) -> String {
        inputs
            .get(index)
            .and_then(|input| input.as_ref())
            .cloned()
            .unwrap_or_else(|| "'()".to_string())
    }

    fn arg_expr(&self, inputs: &[Option<String>]) -> String {
        match self.arg_mode {
            ArgMode::List => Self::input_expr(inputs, 1),
            ArgMode::Fixed(arg_count) => {
                let args = (1..=arg_count.get())
                    .map(|index| Self::input_expr(inputs, index))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("(list {args})")
            }
        }
    }
}

impl node::Node for Apply {
    /// Inputs:
    ///
    /// 1. A function value. Receiving this triggers evaluation.
    /// 2. Either a list of arguments, or one separate input per argument.
    fn n_inputs(&self, _ctx: node::MetaCtx) -> usize {
        self.arg_mode.node_input_count()
    }

    /// The result of function application.
    fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
        1
    }

    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        let inputs = ctx.inputs();

        let function = inputs.get(0).and_then(|opt| opt.as_ref());
        let args = self.arg_expr(inputs);
        let expr = function
            .map(|f| format!("(apply {f} {args})"))
            .unwrap_or_else(|| "'()".to_string());
        node::parse_expr(&expr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconfigured_returns_cleared_inputs_for_list_and_fixed_modes() {
        let apply = Apply::default();
        assert_eq!(apply.reconfigured(None).unwrap().1, Vec::<usize>::new());
        assert_eq!(apply.reconfigured(Some(2)).unwrap().1, vec![1]);

        let apply = Apply::fixed(4).unwrap();
        assert_eq!(apply.reconfigured(Some(4)).unwrap().1, Vec::<usize>::new());
        assert_eq!(apply.reconfigured(Some(2)).unwrap().1, vec![3, 4]);
        assert_eq!(apply.reconfigured(Some(6)).unwrap().1, Vec::<usize>::new());
        assert_eq!(apply.reconfigured(None).unwrap().1, vec![1, 2, 3, 4]);
    }

    #[test]
    fn fixed_arg_count_validation() {
        assert!(Apply::fixed(0).is_err());
        assert!(Apply::fixed(Apply::MAX_FIXED_ARGS + 1).is_err());
        assert_eq!(
            Apply::fixed(3)
                .ok()
                .and_then(|apply| apply.fixed_arg_count()),
            Some(3)
        );
    }

    #[test]
    fn content_addr_changes_with_arg_mode() {
        let list = gantz_ca::content_addr(&Apply::default());
        let fixed = gantz_ca::content_addr(&Apply::fixed(2).unwrap());
        assert_ne!(list, fixed);
    }

    #[test]
    fn invalid_fixed_arg_count_deserialization_fails() {
        let json = r#"{"arg_mode":{"Fixed":0}}"#;
        assert!(serde_json::from_str::<Apply>(json).is_err());
    }
}
