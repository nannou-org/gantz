# gantz

An environment for creative systems.

gantz is inspired by a desire for a more flexible, high-performance,
open-source alternative to graphical programming environments such as Max/MSP,
Touch Designer, Houdini and others. <sup>Named after [*gantz graf*][gantz_graf].</sup>

Goals include:

- **The zen of the empty graph**. A feeling of endless creative possibility
  when you open gantz.
- **Interactive programming, realtime feedback**. Modify the graph while it
  runs and immediately feel the results.
- **Functions as values**. Inspired by functional programming, explore how
  higher-order functions can enable [higher-order
  patterns](https://slab.org/2025/02/01/tidal-a-history-in-types/).

*NOTE: gantz is currently a research project and is not ready for any kind of
real-world use.*

## Crates

The following gantz crates are included in this repo.

| Crate | Release | Description |
|---|---|---|
| **`gantz_core`** | [![Crates.io](https://img.shields.io/crates/v/gantz_core.svg)](https://crates.io/crates/gantz_core) | The core node and graph abstractions. |
| **`gantz_std`** | [![Crates.io](https://img.shields.io/crates/v/gantz_std.svg)](https://crates.io/crates/gantz_std) | A standard library of commonly useful nodes. |
| **`gantz_egui`** | [![Crates.io](https://img.shields.io/crates/v/gantz_egui.svg)](https://crates.io/crates/gantz_egui) | UI traits and widgets that make up the gantz GUI. |
| **`bevy_gantz`** | [![Crates.io](https://img.shields.io/crates/v/bevy_gantz.svg)](https://crates.io/crates/bevy_gantz) | A bevy plugin for gantz. |
| **`gantz`** | [![Crates.io](https://img.shields.io/crates/v/gantz.svg)](https://crates.io/crates/gantz) | The top-level gantz app. |

## Design Overview

gantz allows for constructing executable directed graphs by composing together
**Nodes**.

### Nodes

**Nodes** are a way to allow users to abstract and encapsulate logic into
smaller, re-usable components, similar to a function in a coded programming
language.

Every **Node** is made up of a number of inputs, a number of outputs, and an
expression that takes the inputs as arguments and returns the outputs in a
list. Values can be anything including numbers, strings, lists, maps,
functions and more.

Nodes can opt-in to state, branching on their outputs, and acting as
entrypoints to the graph.

### Graphs

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
