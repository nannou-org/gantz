//! Ephemeral node-interaction *actions*, in a wire-encodable form.
//!
//! Graph edits are durable actions: they mint content-addressed commits and
//! replicate via the registry sync machinery. Everything else a node UI can
//! do reduces - by construction of [`NodeCtx`][crate::NodeCtx] - to exactly
//! two ephemeral primitives, captured here in serialisable form:
//!
//! - a **VM-state write** ([`Action::SetState`]): the only state-write
//!   channel available to a `NodeUi` implementation is
//!   [`NodeCtx::update_value`][crate::NodeCtx::update_value], which records a
//!   [`StateWrite`] as a side effect (see
//!   [`NodeCtx::update_value_local`][crate::NodeCtx::update_value_local] to
//!   opt out);
//! - an **evaluation trigger** ([`Action::Eval`]): push/pull entrypoints are
//!   content-addressed and re-derivable from their [`Source`]s, so a remote
//!   peer holding the same graph rebuilds the identical entry fn - no code
//!   travels.
//!
//! Actions are addressed by node-index [`path`](gantz_core::node::Id)s,
//! which are only meaningful relative to a specific graph: transport layers
//! anchor each action to the graph address it was issued against and drop it
//! on mismatch. Delivery is fire-and-forget; a lost action is no worse than
//! an app restart (VM state is not persisted either).

use gantz_core::compile::entrypoint::{self, EvalKind, EvalSource};
use gantz_core::node;
use serde::{Deserialize, Serialize};
use steel::SteelVal;

/// An ephemeral node-interaction action.
///
/// The complete algebra of what a `NodeUi` can do to the VM: state writes
/// and evaluation triggers, plus a reserved extension variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    /// A live VM-state write, optionally fused with the push-eval it
    /// triggered in the same frame (fused so "set then evaluate" stays
    /// atomic on the remote side - an eval arriving before its value would
    /// fire on stale state).
    SetState {
        path: Vec<node::Id>,
        value: Value,
        eval: Option<Source>,
    },
    /// A push/pull evaluation with no accompanying state write (e.g. a
    /// `bang` click).
    Eval { sources: Vec<Source> },
    /// Reserved escape hatch for custom node actions, tagged with a
    /// wire-stable string (the [`gantz_nodetag`]-style discipline; `TypeId`s
    /// are not stable across builds). Nothing emits this today; decoders
    /// must log and drop unknown tags.
    ///
    /// [`gantz_nodetag`]: https://docs.rs/gantz_nodetag
    Custom { tag: String, data: Vec<u8> },
}

/// A serialisable [`EvalSource`]: one evaluation source within a graph tree.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// Full path to the node from root.
    pub path: Vec<node::Id>,
    /// Whether the source pushes or pulls evaluation.
    pub kind: Kind,
    /// Which connections participate in evaluation.
    pub conns: node::Conns,
}

/// A serialisable [`EvalKind`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    Push,
    Pull,
}

/// A VM-state value in wire-encodable form.
///
/// Covers the value shapes interactive nodes actually write (numbers, lists
/// and friends); rich runtime values (closures, ports, custom types) are
/// deliberately unsupported - a write of one simply isn't captured.
///
/// `Int` and `Num` are distinct on purpose: nodes branch on the steel
/// variant (e.g. the `number` dialer renders integer vs float dialers), so a
/// lossy `serde_json`-style int-to-float bridge would corrupt behaviour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Num(f64),
    Char(char),
    Str(String),
    List(Vec<Value>),
}

/// A VM-state write recorded by
/// [`NodeCtx::update_value`][crate::NodeCtx::update_value].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateWrite {
    pub path: Vec<node::Id>,
    pub value: Value,
}

/// Response payload: a node UI wrote VM state via
/// [`NodeCtx::update_value`][crate::NodeCtx::update_value] this frame.
///
/// Emitted head-tagged alongside the node's other payloads. Applications
/// without a use for it may drop it silently; the collaborative-session
/// layer broadcasts it to peers as an [`Action::SetState`].
#[derive(Clone, Debug)]
pub struct StateWritten(pub StateWrite);

/// The error returned when a [`SteelVal`] has no [`Value`] representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnsupportedValue;

impl std::fmt::Display for UnsupportedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "steel value has no wire-encodable representation")
    }
}

impl std::error::Error for UnsupportedValue {}

impl TryFrom<&SteelVal> for Value {
    type Error = UnsupportedValue;
    fn try_from(val: &SteelVal) -> Result<Self, Self::Error> {
        Ok(match val {
            SteelVal::Void => Value::Unit,
            SteelVal::BoolV(b) => Value::Bool(*b),
            SteelVal::IntV(i) => Value::Int(*i as i64),
            SteelVal::NumV(n) => Value::Num(*n),
            SteelVal::CharV(c) => Value::Char(*c),
            SteelVal::StringV(s) => Value::Str(s.to_string()),
            SteelVal::ListV(l) => {
                Value::List(l.iter().map(Value::try_from).collect::<Result<_, _>>()?)
            }
            _ => return Err(UnsupportedValue),
        })
    }
}

impl From<Value> for SteelVal {
    fn from(v: Value) -> Self {
        match v {
            Value::Unit => SteelVal::Void,
            Value::Bool(b) => SteelVal::BoolV(b),
            Value::Int(i) => SteelVal::IntV(i as isize),
            Value::Num(n) => SteelVal::NumV(n),
            Value::Char(c) => SteelVal::CharV(c),
            Value::Str(s) => SteelVal::StringV(s.into()),
            Value::List(l) => SteelVal::ListV(l.into_iter().map(SteelVal::from).collect()),
        }
    }
}

impl From<EvalKind> for Kind {
    fn from(kind: EvalKind) -> Self {
        match kind {
            EvalKind::Push => Kind::Push,
            EvalKind::Pull => Kind::Pull,
        }
    }
}

impl From<Kind> for EvalKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Push => EvalKind::Push,
            Kind::Pull => EvalKind::Pull,
        }
    }
}

impl From<EvalSource> for Source {
    fn from(src: EvalSource) -> Self {
        Self {
            path: src.path,
            kind: src.kind.into(),
            conns: src.conns,
        }
    }
}

impl From<Source> for EvalSource {
    fn from(src: Source) -> Self {
        Self {
            path: src.path,
            kind: src.kind.into(),
            conns: src.conns,
        }
    }
}

/// Rebuild the content-addressed [`Entrypoint`](entrypoint::Entrypoint) an
/// [`Action::Eval`]'s sources describe. The id (and thus the generated entry
/// fn name) is identical to the emitting peer's, because entrypoints are
/// content-addressed over their sorted source set.
pub fn entrypoint(sources: impl IntoIterator<Item = Source>) -> entrypoint::Entrypoint {
    entrypoint::from_sources(sources.into_iter().map(EvalSource::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_round_trips_through_steel() {
        let values = [
            Value::Unit,
            Value::Bool(true),
            Value::Int(-42),
            Value::Num(1.5),
            Value::Char('g'),
            Value::Str("gantz".to_string()),
            Value::List(vec![Value::Int(1), Value::List(vec![Value::Num(2.0)])]),
        ];
        for value in values {
            let steel = SteelVal::from(value.clone());
            assert_eq!(Value::try_from(&steel).unwrap(), value);
        }
    }

    // Int and Num must stay distinct through the round trip: nodes branch on
    // the steel variant.
    #[test]
    fn value_preserves_int_vs_num() {
        assert!(matches!(SteelVal::from(Value::Int(1)), SteelVal::IntV(1)));
        assert!(matches!(
            SteelVal::from(Value::Num(1.0)),
            SteelVal::NumV(n) if n == 1.0
        ));
    }

    // Unsupported runtime values (closures etc.) are skipped, not mangled.
    #[test]
    fn unsupported_values_are_rejected() {
        let steel = SteelVal::SymbolV("nope".into());
        assert_eq!(Value::try_from(&steel), Err(UnsupportedValue));
        // A list containing an unsupported value is rejected wholesale.
        let steel = SteelVal::ListV(
            [SteelVal::IntV(1), SteelVal::SymbolV("nope".into())]
                .into_iter()
                .collect(),
        );
        assert_eq!(Value::try_from(&steel), Err(UnsupportedValue));
    }

    // A round-tripped Source rebuilds the IDENTICAL content-addressed
    // entrypoint - the property remote eval relies on.
    #[test]
    fn eval_sources_rebuild_the_identical_entrypoint() {
        let ep = entrypoint::push(vec![3, 1], 2);
        let sources: Vec<Source> = ep.0.iter().cloned().map(Source::from).collect();
        let rebuilt = entrypoint(sources);
        assert_eq!(rebuilt, ep);
        assert_eq!(rebuilt.id(), ep.id());
    }
}
