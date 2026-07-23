//! The [`NodeUi`](gantz_egui::NodeUi) implementations for the DSP node set,
//! one submodule per node, mirroring [`crate::node`].
//!
//! Node *behaviour* (fields, `Node`, `NodeDsp`) lives in [`crate::node`];
//! only the egui surface lives here, reaching the nodes through their public
//! accessors.

pub mod bus;
pub mod lag;
pub mod out;
pub mod pack;
pub mod play_buf;
pub mod scope_out;
pub mod sin_osc;
pub mod sum;
pub mod unit;
pub mod unpack;
