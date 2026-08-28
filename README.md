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

gantz runs natively, but you can also try it [in the browser](https://nannou-org.github.io/gantz).

## Crates

The following gantz crates are included in this repo.

This repo is **multi-license**. Most crates are dual-licensed `MIT OR Apache-2.0`.
The DSP crates (`gantz_plyphon`, `bevy_gantz_plyphon`) and the `gantz` application
are `GPL-3.0-or-later`, since they build on
[plyphon](https://github.com/mitchmindtree/plyphon) (GPL-3.0). The pattern crate
(`gantz_pattern`) is also `GPL-3.0-or-later`. Each crate carries its own license
file(s).

| Crate | Release | License | Description |
|---|---|---|---|
| **`gantz_base`** | [![Crates.io](https://img.shields.io/crates/v/gantz_base.svg)](https://crates.io/crates/gantz_base) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | Embedded base node export for gantz. |
| **`gantz_ca`** | [![Crates.io](https://img.shields.io/crates/v/gantz_ca.svg)](https://crates.io/crates/gantz_ca) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | The gantz content addressing abstractions. |
| **`gantz_ca_derive`** | [![Crates.io](https://img.shields.io/crates/v/gantz_ca_derive.svg)](https://crates.io/crates/gantz_ca_derive) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | Derive macro for the `CaHash` content-addressing trait. |
| **`gantz_nodetag`** | [![Crates.io](https://img.shields.io/crates/v/gantz_nodetag.svg)](https://crates.io/crates/gantz_nodetag) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | Wire tags identifying gantz node types across serialization formats. |
| **`gantz_nodetag_derive`** | [![Crates.io](https://img.shields.io/crates/v/gantz_nodetag_derive.svg)](https://crates.io/crates/gantz_nodetag_derive) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | Derive macro for the `NodeTag` trait. |
| **`gantz_core`** | [![Crates.io](https://img.shields.io/crates/v/gantz_core.svg)](https://crates.io/crates/gantz_core) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | The core node and graph abstractions. |
| **`gantz_std`** | [![Crates.io](https://img.shields.io/crates/v/gantz_std.svg)](https://crates.io/crates/gantz_std) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | A standard library of commonly useful nodes. |
| **`gantz_format`** | [![Crates.io](https://img.shields.io/crates/v/gantz_format.svg)](https://crates.io/crates/gantz_format) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | Human-readable text format for gantz graph registries. |
| **`gantz_ui`** | [![Crates.io](https://img.shields.io/crates/v/gantz_ui.svg)](https://crates.io/crates/gantz_ui) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | Declarative UI tree model and value codecs for user-defined gantz GUIs. |
| **`gantz_egui`** | [![Crates.io](https://img.shields.io/crates/v/gantz_egui.svg)](https://crates.io/crates/gantz_egui) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | UI traits and widgets that make up the gantz GUI. |
| **`gantz_collab`** | [![Crates.io](https://img.shields.io/crates/v/gantz_collab.svg)](https://crates.io/crates/gantz_collab) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | Peer-to-peer collaborative session networking for gantz. |
| **`bevy_gantz`** | [![Crates.io](https://img.shields.io/crates/v/bevy_gantz.svg)](https://crates.io/crates/bevy_gantz) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | A bevy plugin for gantz. |
| **`bevy_gantz_egui`** | [![Crates.io](https://img.shields.io/crates/v/bevy_gantz_egui.svg)](https://crates.io/crates/bevy_gantz_egui) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | Bevy and egui integration for gantz. |
| **`bevy_gantz_collab`** | [![Crates.io](https://img.shields.io/crates/v/bevy_gantz_collab.svg)](https://crates.io/crates/bevy_gantz_collab) | ![MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue.svg) | Bevy integration of gantz's peer-to-peer collaborative sessions. |
| **`gantz_plyphon`** | [![Crates.io](https://img.shields.io/crates/v/gantz_plyphon.svg)](https://crates.io/crates/gantz_plyphon) | ![GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blueviolet.svg) | DSP nodes + synthdef compiler deriving [plyphon](https://github.com/mitchmindtree/plyphon) synthdefs from gantz graphs. |
| **`bevy_gantz_plyphon`** | [![Crates.io](https://img.shields.io/crates/v/bevy_gantz_plyphon.svg)](https://crates.io/crates/bevy_gantz_plyphon) | ![GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blueviolet.svg) | Bevy + plyphon audio runtime for gantz (cpal stream, synth driver). |
| **`gantz_pattern`** | [![Crates.io](https://img.shields.io/crates/v/gantz_pattern.svg)](https://crates.io/crates/gantz_pattern) | ![GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--2.0--or--later-blueviolet.svg) | Composable pattern generation as a Steel module. |
| **`gantz`** | [![Crates.io](https://img.shields.io/crates/v/gantz.svg)](https://crates.io/crates/gantz) | ![GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blueviolet.svg) | The top-level gantz app. |

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

One graph may be compiled by more than one backend. Control-rate nodes compile
to steel and run from the main GUI thread. Nodes with harder realtime
constraints - audio DSP today, GPU shaders in future - carry a second,
specialised representation and are derived into a subgraph that runs elsewhere.
See [Domains](#domains) below.

### Content addressing

Graphs, nodes and values are content-addressed. Each is identified by the hash
of its structure rather than by a name or an index. Identity survives being
copied between the registry, the text format and other peers, so structurally
shared graphs are cheap and merging concurrent edits is easy.

### Domains

The core graph and VM know nothing about any particular application domain. A
*domain* layers its own nodes, its own lazily-compiled steel module and its
own starter graphs on top of the core. Custom gantz apps can pick and choose
between domains, or write their own. Today, the gantz app included with this
repo includes:

- **DSP**. Connected audio subgraphs of a patch are compiled into [plyphon]
  synthdefs that run off the GUI thread, so sample-accurate audio is never
  scheduled by the VM. A nested graph compiles to a single template shared by
  its instances, and its params stay moddable while the audio runs.
- **Pattern**. A pattern is a function from a span of rational time to the
  events along it, inspired by TidalCycles. Being plain functions, patterns
  compose as higher-order values along edges, and a graph lifted into a function
  is as composable as any other.
- **Collaboration**. A graph and everything it depends on is shared between
  peers over [iroh], with concurrent edits merged using the sync model that
  content addressing provides.

### Graph GUIs

A graph can describe its own GUI as a *value*. Just as the `inlet` and `outlet`
markers declare a graph's sockets, a `gui` marker declares its GUI: a tree of
plain data, evaluated by the graph itself and interpreted by the host each
frame.

A marker names the surface it fills, and widgets bind to node state by path,
the idea being to enable custom UI for graphs without the need for any
dedicated host code. The tree model is free of any GUI toolkit, so a graph's
GUI travels with it wherever the graph goes. In this repo, we interpret the GUI
tree using `egui`.

[gantz_graf]: https://youtu.be/ev3vENli7wQ
[steel]: https://github.com/mattwparas/steel
[iroh]: https://github.com/n0-computer/iroh
[plyphon]: https://github.com/mitchmindtree/plyphon
