//! Embedded base node export for gantz.
//!
//! Ships the `base.gantz` file as a compile-time constant so downstream
//! crates don't need fragile relative `include_bytes!` paths.

/// Raw bytes of the baked-in base `.gantz` export, embedded at compile time.
pub const BYTES: &[u8] = include_bytes!("../base.gantz");
