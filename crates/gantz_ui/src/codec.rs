//! Per-runtime codecs between the abstract value model and value types.
//!
//! Each codec is four total free functions. `lower` maps a runtime value
//! into [`crate::sexpr::SExpr`]. `raise` maps an `SExpr` back. `decode` and
//! `encode` compose them with the shared decoder and encoder. Adding a
//! backend means writing one small total lowering. The model, decoder and
//! encoder are shared.

#[cfg(feature = "datum")]
pub mod datum;
#[cfg(feature = "steel")]
pub mod steel;
