# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1](https://github.com/nannou-org/gantz/compare/gantz-v0.3.0...gantz-v0.3.1) - 2026-08-26

### Added

- *(gantz_plyphon)* add ringmod and waveshape demos
- *(gantz)* register the pm node and use it in the demo
- *(gantz)* wire the pattern domain base file and steel module
- *(gantz_egui)* resolve gui markers via the environment
- *(gantz_egui)* add the Gui marker node
- *(gantz)* register await and sleep in the app node set

### Other

- *(gantz_pattern)* rename the pm node to pmini
- *(gantz)* pin demo-pattern partial evals as silent
- *(gantz_pattern)* address review on naming and comments
- *(gantz)* pin the #:require pipeline end to end

## [0.3.0](https://github.com/nannou-org/gantz/compare/gantz-v0.2.3...gantz-v0.3.0) - 2026-07-23

### Added

- *(gantz)* collab cargo feature gating the p2p networking
- *(gantz)* wire up live collaborative sessions
- *(app,plyphon)* [**breaking**] register unit nodes, migrate ~sinosc/~lag to the table
- [**breaking**] the working graph is data
- [**breaking**] builtins are data
- *(core,egui)* the node codec seam
- *(ca,collab)* [**breaking**] concrete Registry, verified relays, RawGraph deleted
- [**breaking**] flip the app stack onto the concrete data registry
- canonical dehydration gate + skip-if-default serde sweep
- *(gantz_plyphon)* the ~playbuf sample-playback node

### Fixed

- *(egui,app)* [**breaking**] session-restored heads missed the instance cache component

### Other

- [**breaking**] purge typed-node CaHash
- [**breaking**] concrete Env, N-free plugins, app node trait retired
- *(egui,format)* [**breaking**] registry-side helpers operate on data
- *(core)* rename dehydrate/hydrate to erase/reify
- *(gantz,bevy_gantz_plyphon)* end-to-end ~playbuf playback + reachability
- *(plyphon,gantz)* graph-addr resolution and app-crate port
- Merge pull request #332 from mitchmindtree/feat/hybrid-dsp-inputs

## [0.2.3](https://github.com/nannou-org/gantz/compare/gantz-v0.2.2...gantz-v0.2.3) - 2026-07-12

### Added

- *(gantz)* register ~sum in the app node set
- *(plyphon,bevy)* instancing is the default lowering for DSP refs
- *(plyphon)* flatten markers for instanced refs and root boundaries
- *(dsp)* driver refcounted shared defs (install once, spawn many)
- *(plyphon)* egui cargo feature gating the GUI impls
- *(core)* AsRefNode reference probe
- *(egui)* cross-source base refs
- *(gantz,egui)* per-source update-base write-back
- *(plyphon)* the plyphon base source
- *(egui)* ref ext integration
- *(core)* add composable builtin specs
- *(gantz)* pop panes out into native OS windows (#271, part 1)
- *(web)* AudioWorklet audio for the gantz website
- *(gantz_plyphon)* ~bus synthdef boundaries with per-region synths
- *(gantz_plyphon)* per-ugen rate (ar/kr) on ~sinosc and ~lag
- *(bevy_gantz_plyphon)* crossfade synth replacement instead of a hard cut
- *(gantz_plyphon)* ~pack/~unpack channel-routing nodes
- *(gantz_plyphon)* multichannel signals as channel-group edges
- *(gantz_plyphon)* ~scopeout branches gating + configurable channels
- *(gantz_plyphon)* add the ~tap DSP->control monitor node
- *(gantz_plyphon)* DSP->control monitor return-path infra
- *(gantz_plyphon)* summarise DSP node state as queued-update count
- *(gantz_plyphon)* .gantz keyword sugar for ~sine/~out/~lag
- *(gantz_plyphon)* queue timestamped control updates in param state
- *(gantz_plyphon)* DSP control inputs + a ~lag node
- *(gantz_plyphon)* DSP param values as node state + combined inspector row
- *(gantz)* wire the DSP crates into the app
- *(gantz_format)* merge-parents clause; ancestry walks follow merge parents
- *(gantz)* compose the per-crate node sugars via NodeSugar
- *(node)* add Hz rate mode to tick!
- *(node)* add self-driven tick! node
- *(egui)* persist graph camera as centre + zoom, not a rect
- *(persist)* log persist duration and on-disk counts
- *(plot)* refine inspector, frame and interaction per review
- *(plot)* refine plot node per review feedback
- *(plot)* add a configurable plot node to gantz_egui

### Fixed

- *(gantz)* drop unused ToNodeDsp import in driver test
- *(gantz)* order-insensitive log layer, document assembly
- *(named-ref)* include `sync` in the content address so toggles persist

### Other

- Merge branch 'master' into feat/dsp-input-summing
- *(bevy,gantz)* instancing runtime coverage
- *(gantz)* compose domain builtins, delete builtin.rs
- native pop-out windows as unbounded single-pass egui contexts
- *(gantz)* nested DSP graph derives and bridges VM state
- *(dsp)* rename the Audio* abstraction to Dsp*
- *(web)* drop the legacy web build, promote AudioWorklet to canonical
- *(gantz_plyphon)* name dsp nodes after their ugens
- *(gantz)* guard DSP nodes' serde round-trip + Steel-inertness
- *(license)* multi-license the repo; require explicit per-crate license
- move NodeTag into dedicated gantz_nodetag crates
- drop typetag from the workspace
- *(gantz)* replace typetag with impl_node_set_serde! ([#181](https://github.com/nannou-org/gantz/pull/181))
- *(gantz)* pin the node serde wire format (Datum + RON)
- *(node)* rename frame! to update!
- *(persist)* serialize egui memory with bincode, not RON
- *(persist)* persist views incrementally, one key per commit
- *(gantz)* extract persistence into a persist module
- *(persist)* write to storage on a background worker
- *(persist)* stagger egui memory onto a separate debounce
- *(persist)* persist registry incrementally
- *(gantz)* order debounced persistence after settle_layout

## [0.2.2](https://github.com/nannou-org/gantz/compare/gantz-v0.2.1...gantz-v0.2.2) - 2026-06-21

### Added

- *(named-ref)* guard against reference cycles when adding a NamedRef
- *(format)* add inline-name export for base.gantz
- *(base)* document every base node socket
- *(base)* enable auto-sync on all demo refs
- *(base)* add demo-all catalog of every base node
- *(base)* add pure-primitive node library with per-category demos

### Fixed

- *(base)* stamp base graphs with a fixed timestamp so reset keeps refs valid
- *(base)* make mod total so a zero divisor cannot panic the app
- *(base)* coerce integer ops, bang both inputs, add demo layouts

## [0.2.1](https://github.com/nannou-org/gantz/compare/gantz-v0.2.0...gantz-v0.2.1) - 2026-06-15

### Added

- *(gantz)* persist native window size between sessions
- *(format)* warn when import clears an absent commit parent
- *(format)* add human/LLM-readable .gantz text format
- *(gui)* thread module artifact and diagnostics to frontends
- *(app)* expose the delay node as a builtin
- add Branch node for conditional output activation
- move FrameBang from gantz_egui to bevy_gantz_egui, drive eval from Bevy system
- add `frame!` node for continuous per-frame evaluation
- add demo graph associations, reset, and UI polish

### Other

- Merge pull request #237 from mitchmindtree/feat/persist-window-size
- *(egui)* remove RON from export/import; clipboard uses .gantz text
- *(gantz)* port typetag gate to the Datum codec
- *(format)* extract gantz_format crate from gantz_egui
- *(format)* normalize structure into graph/commits/names tables
- *(bevy)* input-addressed VM sync replaces scattered recompile paths
- apply cargo fmt
- *(gui)* merge CompiledModule into Module; render errors separately
