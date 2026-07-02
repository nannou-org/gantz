# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.3](https://github.com/nannou-org/gantz/compare/gantz-v0.2.2...gantz-v0.2.3) - 2026-07-02

### Added

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

- *(named-ref)* include `sync` in the content address so toggles persist

### Other

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
