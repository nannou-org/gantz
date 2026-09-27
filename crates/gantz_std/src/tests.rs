use gantz_core::Node;
use std::fmt::Debug;

mod bang;
mod log;
mod number;

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}
