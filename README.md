# gantz

An environment for creative systems.

gantz is inspired by a desire for a more flexible, high-performance,
open-source alternative to graphical programming environments such as Max/MSP,
Touch Designer, Houdini and others. <sup>Named after [*gantz graf*][gantz_graf].</sup>

Goals include:

- **The zen of the empty graph**. A feeling of endless creative possibility
  when you open gantz.
- **Interactive programming with realtime feedback**. Modify the graph while it
  runs and immediately feel the results.
- **Functions as values**. Take inspiration from functional programming and
  explore how higher-order functions can enable [higher-order
  patterns](https://slab.org/2025/02/01/tidal-a-history-in-types/).

*NOTE: gantz is currently a research project and is not ready for any kind of
real-world use.*

## Crates

The following gantz crates are included in this repo.

| Crate | Release | Description |
|---|---|---|
| **gantz_core** | [![crates.io][gantz_core]][gantz_core_svg] | The core node and graph abstractions. |
| **gantz_std** | [![crates.io][gantz_std]][gantz_std_svg] | A standard library of commonly useful nodes. |
| **gantz_egui** | [![crates.io][gantz_egui]][gantz_egui_svg] | UI traits and widgets that make up the gantz GUI. |
| **bevy_gantz** | [![crates.io][bevy_gantz]][bevy_gantz_svg] | **TODO** A bevy plugin for gantz. |
| **gantz** | [![crates.io][gantz]][gantz_svg] | **TODO** The top-level gantz app. |

## Design Overview

gantz allows for constructing executable directed graphs by composing together
**Nodes**.

**Nodes** are a way to allow users to abstract and encapsulate logic into
smaller, re-usable components, similar to a function in a coded programming
language.

Every **Node** is made up of a number of inputs, a number of outputs, and an
expression that takes the inputs as arguments and returns the outputs in a
list. Values can be anything including numbers, strings, lists, maps,
functions and more.

Nodes can opt-in to state, branching on their outputs, and acting as
entrypoints to the graph.

**Graphs** describe the composition of one or more nodes. A graph may contain
one or more nested graphs represented as nodes, forming the main method of
abstraction within gantz.

Graphs are compiled to [steel], an embeddable scheme written in Rust designed
for embedding in Rust applications. This allows for fast dynamic evaluation,
while providing the option to specialise node implementations using native Rust
functions where necessary.

The generated steel code is designed solely for interaction from the main GUI
thread. For realtime audio DSP, GPU shaders, and other domains with unique
constraints, a specialised subgraph will be derived from the top-level gantz
graph.

See the `gantz_core/tests` directory for some very basic, early proof-of-concept
tests.

[gantz_graf]: https://youtu.be/ev3vENli7wQ
[steel]: https://github.com/mattwparas/steel
[gantz_core]: https://crates.io/crates/gantz_core
[gantz_core_svg]: https://img.shields.io/crates/v/gantz_core.svg
[gantz_std]: https://crates.io/crates/gantz_std
[gantz_std_svg]: https://img.shields.io/crates/v/gantz_std.svg
[gantz_egui]: https://crates.io/crates/gantz_egui
[gantz_egui_svg]: https://img.shields.io/crates/v/gantz_egui.svg
