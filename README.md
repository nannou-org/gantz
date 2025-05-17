# gantz [![Actions Status](https://github.com/nannou-org/gantz/workflows/gantz/badge.svg)](https://github.com/nannou-org/gantz/actions) [![Crates.io](https://img.shields.io/crates/l/gantz.svg)](https://github.com/nannou-org/gantz/blob/master/LICENSE-MIT)

A crate for creating and evaluating executable directed graphs at runtime. In
other words, gantz allows users to compose programs described by interconnected
nodes on the fly.

Gantz is inspired by a desire for a more flexible, high-performance, open-source
alternative to graphical programming environments such as Max/MSP, Touch
Designer, Houdini and others. <sup>Named after
[*gantz graf*][gantz_graf].</sup>

*NOTE: gantz is currently a research project and is not ready for any kind of
real-world use.*

## Design Overview

Gantz allows for constructing executable directed graphs by composing together
**Nodes**.

**Nodes** are a way to allow users to abstract and encapsulate logic into
smaller, re-usable components, similar to a function in a coded programming
language.

Every **Node** is made up of the following:

- Any number of inputs, where each input is some value.
- Any number of outputs, where each output is some value.
- An expression or function that takes the inputs as arguments and returns the
  outputs in a list.

**Graphs** describe the composition of one or more nodes. A graph may contain
one or more nested graphs represented as nodes, forming the main method of
abstraction within gantz.

Graphs are compiled to [steel], an embeddable scheme written in Rust designed
for embedding in Rust applications. This allows for fast dynamic evaluation,
while providing the option to specialise node implementations using native Rust
functions where necessary.

See the `gantz_core/tests` directory for some very basic, early proof-of-concept
tests.

## Included Crates

### gantz_core [![Crates.io][1]][2] [![docs.rs][3]][4]

Contains the core traits and items necessary for any gantz implementation. The
current approach heavily revolves around steel code generation, however this
crate may get generalised in the future to allow for more easily targeting other
languages.

## Goals

- [x] A simple function for creating nodes from steel expressions.
- [x] Allow for handling generics and trait objects within custom nodes.
- [x] `Serialize` and `Deserialize` for nodes and graphs via serde and typetag.
- [x] Project workspace creation.
- [x] **Push** evaluation through the graph.
- [x] **Pull** evaluation through the graph.
- [x] Simultaneous push and pull evaluation from multiple nodes.
- [x] Stateless node codegen.
- [x] Stateful node codegen.
- [x] Implement `Node` for `Graph`.
- [ ] Conditional evaluation #21.
- [ ] Evaluation boundaries #22.
- [ ] Dynamic node I/O configurations #31.
- [ ] A derive macro for generating node types from existing Rust `fn`s.

After each of these goals are met, the plan is to use this foundation to create
some higher-level tooling along the lines of the following:

- [ ] A GUI for creating, editing and saving graphs and custom nodes at runtime.
- [ ] Node packaging and sharing tools, likely built on cargo and crates.io.
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

[gantz_graf]: https://youtu.be/ev3vENli7wQ
[steel]: https://github.com/mattwparas/steel
[1]: https://img.shields.io/crates/v/gantz_core.svg
[2]: https://crates.io/crates/gantz_core
[3]: https://docs.rs/gantz_core/badge.svg
[4]: https://docs.rs/gantz_core/
