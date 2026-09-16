//! The [`NodeUi`](gantz_egui::NodeUi) implementations for the DSP node set,
//! one submodule per node, mirroring [`crate::node`].
//!
//! Node behaviour, the fields and the `Node` and `NodeDsp` impls, lives in
//! [`crate::node`]. Only the egui surface lives here. It reaches the nodes
//! through their public accessors.

pub mod bus;
pub mod out;
pub mod pack;
pub mod play_buf;
pub mod scope_out;
pub mod sum;
pub mod unit;
pub mod unpack;
