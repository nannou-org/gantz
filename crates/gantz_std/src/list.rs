use gantz_core::node::{ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// Collect the connected inputs into a list, in socket order.
///
/// The input count is configurable, so a graph can gather any number of
/// values without an `expr`. An unconnected input is skipped rather than
/// contributing a placeholder, so the list holds exactly what is wired in.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, NodeTag)]
pub struct List {
    #[serde(default = "default_count", skip_serializing_if = "is_default_count")]
    count: usize,
}

impl List {
    /// The number of inputs a fresh `list` starts with.
    pub const DEFAULT_COUNT: usize = 2;
    /// The largest input count.
    pub const MAX_COUNT: usize = 16;

    /// The number of inputs.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Set the input count, clamped to `1..=MAX_COUNT`. It is structural and
    /// affects the content address, since it changes the node's sockets.
    pub fn set_count(&mut self, count: usize) {
        self.count = count.clamp(1, Self::MAX_COUNT);
    }
}

impl Default for List {
    fn default() -> Self {
        List {
            count: default_count(),
        }
    }
}

impl gantz_core::Node for List {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        self.count
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        let items: Vec<&str> = ctx.inputs().iter().flatten().map(String::as_str).collect();
        gantz_core::node::parse_expr(&format!("(list {})", items.join(" ")))
    }
}

fn default_count() -> usize {
    List::DEFAULT_COUNT
}

fn is_default_count(count: &usize) -> bool {
    *count == default_count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::Node;
    use gantz_core::node::Conns;

    fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
        None
    }

    fn expr_of(list: &List, inputs: &[Option<String>]) -> String {
        let outputs = Conns::connected(1).unwrap();
        let ctx = ExprCtx::new(&no_lookup, &[0], inputs, &outputs);
        list.expr(ctx).unwrap().to_string()
    }

    #[test]
    fn connected_inputs_in_socket_order() {
        let mut list = List::default();
        list.set_count(3);
        let all = [Some("a".into()), Some("b".into()), Some("c".into())];
        assert_eq!(expr_of(&list, &all), "(list a b c)");
        let some = [Some("a".into()), None, Some("c".into())];
        assert_eq!(expr_of(&list, &some), "(list a c)");
        assert_eq!(expr_of(&list, &[None, None, None]), "(list)");
    }

    #[test]
    fn count_is_clamped_and_structural() {
        let ctx = MetaCtx::new(&no_lookup);
        let mut list = List::default();
        assert_eq!(list.n_inputs(ctx), List::DEFAULT_COUNT);
        list.set_count(0);
        assert_eq!(list.count(), 1);
        list.set_count(100);
        assert_eq!(list.count(), List::MAX_COUNT);
        assert_eq!(list.n_inputs(ctx), List::MAX_COUNT);
    }
}
