# gantz-derive [![Crates.io](https://img.shields.io/crates/v/gantz-derive.svg)](https://crates.io/crates/gantz-derive) [![Crates.io](https://img.shields.io/crates/l/gantz-derive.svg)](https://github.com/nannou-org/gantz/blob/master/LICENSE-MIT) [![docs.rs](https://docs.rs/gantz-derive/badge.svg)](https://docs.rs/gantz-derive/)

# NOTE: This crate was an old approach that no longer seems to be the best way forward. It may be repurposed for the new direction in the future.

A suite of procedural macros for the gantz crate.

Currently includes:

- **GantzNode**: simplifies the creation of gantz nodes by generating code
  necessary for interoperation with the gantz graph.
- **GantzNode_**: The same as **GantzNode** but for use internally within the
  gantz crate itself.
