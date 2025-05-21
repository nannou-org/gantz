//! Top-level node trait and implementations.

use dyn_hash::DynHash;

/// A top-level blanket trait providing trait object serialization.
#[typetag::serde(tag = "type")]
pub trait Node: DynHash + gantz_core::Node + gantz_egui::NodeUi {}

dyn_hash::hash_trait_object!(Node);

// core nodes
#[typetag::serde]
impl Node for gantz_core::node::Expr {}

// std nodes
#[typetag::serde]
impl Node for gantz_std::ops::Add {}
#[typetag::serde]
impl Node for gantz_std::Bang {}
#[typetag::serde]
impl Node for gantz_std::Log {}
#[typetag::serde]
impl Node for gantz_std::Number {}

// TODO: Remove the above in favour of this if a solution lands:
// https://github.com/dtolnay/typetag/issues/1
// #[typetag::serde]
// impl<T> Node for T where T: Hash + gantz_core::Node + NodeUi + SerdeNode {}
