# gantz [![Build Status](https://travis-ci.org/nannou-org/gantz.svg?branch=master)](https://travis-ci.org/nannou-org/gantz) [![Crates.io](https://img.shields.io/crates/v/gantz.svg)](https://crates.io/crates/gantz) [![Crates.io](https://img.shields.io/crates/l/gantz.svg)](https://github.com/nannou-org/gantz/blob/master/LICENSE-MIT) [![docs.rs](https://docs.rs/gantz/badge.svg)](https://docs.rs/gantz/)

A crate for creating and evaluating executable directed graphs at runtime. In
other words, gantz allows users to compose programs described by interconnected
nodes on the fly.

Gantz is inspired by a desire for a more flexible, high-performance, open-source
alternative to graphical programming environments such as Max/MSP, Touch
Designer, Houdini and others. <sup>Named after
[*gantz graf*](https://youtu.be/ev3vENli7wQ).</sup>

## Goals

- [x] A simple way of creating custom nodes from rust code using `derive`.
- [x] Solve handling of generics and trait objects within custom nodes.
- [x] `Serialize` and `Deserialize` implementations via serde and typetag.
- [x] Project workspace creation.
- [x] **Push** evaluation through the graph.
- [ ] **Pull** evaluation through the graph (#16).
- [x] Stateless node codegen.
- [ ] Stateful node codegen #19.
- [ ] Conditional evaluation #21.
- Provide a suite of commonly required "std" nodes out of the box:
  - [ ] Primitive types and casts.
  - [ ] Mappings to `std::ops`: `Add`, `Sub`, `Mul`, `Div`, etc.
  - [ ] `std::fmt` nodes: `Debug`, `PrettyDebug`, `Display`.
  - [ ] `Vec` constructors and methods.
  - [ ] `String` constructors and methods.
  - [ ] Conversion functions: `Into`, `From`, `FromStr`, `FromIterator`, etc.
  - [ ] A `DeStructure` node that allows de-structuring types into their fields.
  - [ ] Timer/Clock node with push and pull variants. Useful for testing rates.

After each of these goals are met, gantz will be integrated into
[**nannou**](https://github.com/nannou-org/nannou) where it will be extended
with higher-level tools including:

- [ ] A GUI for creating, editing and saving graphs and custom nodes at runtime.
- [ ] Node packaging and sharing tools.
- [ ] A suite of nodes providing an interface to nannou's cross-platform support
  for a wide range of protocols and I/O:
  - [ ] Windowing and input events.
  - [ ] Phasers and signals.
  - [ ] Audio input, output, processing and device management.
  - [ ] 2D/3D geometry, graphics and shaders.
  - [ ] Video input and processing.
  - [ ] Networking (UDP and TCP).
  - [ ] OSC.
  - [ ] Lighting, lasers & control: DMX (via sACN), CITP (& CAEX), Ether-Dream.
  - [ ] GPU general compute.
  - [ ] General file reading and writing.

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

**Contributions**

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
