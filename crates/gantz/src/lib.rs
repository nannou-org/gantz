#[doc(inline)]
pub use gantz_core as core;
#[doc(inline)]
pub use gantz_egui as egui;
#[doc(inline)]
pub use gantz_std as std;

use dyn_hash::DynHash;
use gantz_core::node::SerdeNode;
use gantz_egui::NodeUi;

/// A top-level blanket trait for [`core::Node`], [`NodeUi`] and [`SerdeNode`].
pub trait Node: DynHash + gantz_core::Node + NodeUi + SerdeNode {}

impl<T> Node for T where T: ::std::hash::Hash + gantz_core::Node + NodeUi + SerdeNode {}

dyn_hash::hash_trait_object!(Node);
