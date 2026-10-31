//! A State node: a value for a GUI control to bind to and edit.

use crate::{Env, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind};
use gantz_core::node::{self, EvalConf, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use steel::SteelVal;

/// A node that holds a value in its state, for a GUI control such as a
/// `toggle`, a `choice` or a `color` to bind to and edit. It outputs its
/// value. A push into its input replaces the value. It starts at its `init`
/// value, or empty without one.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
pub struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    init: Option<Init>,
}

/// The value a [`State`] starts at.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Init {
    Bool(bool),
    Number(f64),
    /// A symbol, such as `line`.
    Symbol(String),
    /// A string, such as `"#ff8800"`.
    Text(String),
}

impl State {
    /// A state that starts at `init`, or empty.
    pub fn new(init: Option<Init>) -> Self {
        Self { init }
    }

    /// The value the state starts at.
    pub fn init(&self) -> Option<&Init> {
        self.init.as_ref()
    }
}

impl Init {
    /// The value as a Steel value.
    pub fn to_steel(&self) -> SteelVal {
        match self {
            Self::Bool(b) => SteelVal::BoolV(*b),
            Self::Number(n) => SteelVal::NumV(*n),
            Self::Symbol(s) => SteelVal::SymbolV(s.as_str().into()),
            Self::Text(s) => SteelVal::StringV(s.as_str().into()),
        }
    }
}

impl PartialEq for Init {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Number(a), Self::Number(b)) => a.to_bits() == b.to_bits(),
            (Self::Symbol(a), Self::Symbol(b)) | (Self::Text(a), Self::Text(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Init {}

impl Hash for Init {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Bool(b) => b.hash(state),
            Self::Number(n) => n.to_bits().hash(state),
            Self::Symbol(s) | Self::Text(s) => s.hash(state),
        }
    }
}

impl gantz_core::Node for State {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    // A GUI edit of the value pushes from this node.
    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        vec![EvalConf::All]
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        super::inspect::store_input_expr(ctx)
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        let init = || self.init.as_ref().map_or(SteelVal::Void, Init::to_steel);
        node::state::init_value_if_absent(ctx.vm(), path, init).unwrap()
    }
}

impl NodeUi for State {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "state".into()
    }

    fn description(&self) -> Option<&str> {
        Some("Hold a value for a GUI control to edit")
    }

    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        super::inspect::value_ui(ctx, uictx)
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        Some(match kind {
            SocketKind::Input => {
                SocketDoc::ty("any").with_description("a new value, which replaces the held one")
            }
            SocketKind::Output => SocketDoc::ty("any").with_description("the held value"),
        })
    }
}
