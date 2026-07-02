# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0](https://github.com/nannou-org/gantz/compare/bevy_gantz_egui-v0.3.1...bevy_gantz_egui-v0.4.0) - 2026-07-02

### Added

- *(gantz_format_derive)* derive macro for NodeTag
- declare node wire tags at their definition sites
- *(gantz_egui)* conflict resolution strategy selector on the merge row
- *(gantz_format)* merge-parents clause; ancestry walks follow merge parents
- *(bevy_gantz_egui)* merge observer and demo arm
- *(bevy_gantz_egui)* own its node sugar via BevySugar
- *(gantz_egui)* add Select all, Cut and Duplicate command shortcuts
- *(node)* add Hz rate mode to tick!
- *(node)* add self-driven tick! node
- *(egui)* persist graph camera as centre + zoom, not a rect
- *(bevy_gantz)* drive recompiles from committed CA; commit at edit sites ([#159](https://github.com/nannou-org/gantz/pull/159))
- *(bevy_gantz)* persist and apply graph descriptions
- *(palette)* place new nodes under the pointer and select them
- *(layout-undo)* record settled layout changes in undo history

### Fixed

- *(demo)* key demo associations by graph name instead of commit

### Other

- move NodeTag into dedicated gantz_nodetag crates
- drop typetag from the workspace
- *(node)* rename frame! to update!
- *(persist)* serialize egui memory with bincode, not RON
- *(persist)* persist views incrementally, one key per commit
- house NodeUi response types in response.rs; rename ValidateCommitted
- *(gantz_egui)* NodeUi methods return changed-aware responses

## [0.3.1](https://github.com/nannou-org/gantz/compare/bevy_gantz_egui-v0.3.0...bevy_gantz_egui-v0.3.1) - 2026-06-21

### Added

- *(named-ref)* extend the reference-cycle guard to paste
- *(named-ref)* guard against reference cycles when adding a NamedRef
- *(egui)* sidebar hamburger + tabbed pane layout
- *(format)* add inline-name export for base.gantz
- *(gui)* inlet/outlet hover docs

### Fixed

- *(egui)* don't restore stale persisted egui zoom_factor
- *(base)* stamp base graphs with a fixed timestamp so reset keeps refs valid

### Other

- *(deps)* bump bevy 0.18->0.19 and adopt the published egui 0.34 stack
- *(gui)* store inlet/outlet docs on the nodes
- *(gui)* address socket-doc review feedback

## [0.3.0](https://github.com/nannou-org/gantz/compare/bevy_gantz_egui-v0.2.0...bevy_gantz_egui-v0.3.0) - 2026-06-15

### Added

- *(egui)* repoint parent refs when a nested graph is renamed to root
- *(egui)* resync propagation + fork cascade for nested graphs
- *(egui)* in-place descent + name breadcrumb for nested graphs
- *(egui)* create nested graphs as synced NamedRefs
- *(format)* add human/LLM-readable .gantz text format
- *(gui)* thread module artifact and diagnostics to frontends
- *(gui)* surface the compile config in the Graph Config pane
- consolidate frame! nodes into a single multi-source entrypoint ([#201](https://github.com/nannou-org/gantz/pull/201))
- move FrameBang from gantz_egui to bevy_gantz_egui, drive eval from Bevy system
- add demo graph associations, reset, and UI polish
- paste nodes at mouse position from context menu
- add right-click context menus to graph scene nodes and background
- add right-click context menus to graph scene
- deterministic HashMap serialization via sorted keys

### Fixed

- *(egui)* capture response payload type identity at emission
- branch named nodes with independent commits

### Other

- *(egui)* collapse per-head views to a single egui_graph::View
- remove inline GraphNode; nested graphs are graph refs
- replace Lowerable trait alias with explicit, minimal bounds
- *(egui)* remove RON from export/import; clipboard uses .gantz text
- *(bevy)* input-addressed VM sync replaces scattered recompile paths
- *(egui)* rename response::Payload to DynResponse; fix doc link
- *(egui)* replace Cmd queue with dynamic response channel
- *(egui)* extract shared frontend ops into gantz_egui::ops
- *(gui)* merge CompiledModule into Module; render errors separately
- align eval/entry terminology in event and codegen pipeline
- clean up entrypoint API and consolidate eval command pipeline
