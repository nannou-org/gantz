# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/nannou-org/gantz/compare/gantz_egui-v0.2.0...gantz_egui-v0.3.0) - 2026-06-15

### Added

- *(egui)* repoint parent refs when a nested graph is renamed to root
- *(egui)* resync propagation + fork cascade for nested graphs
- *(egui)* in-place descent + name breadcrumb for nested graphs
- *(egui)* create nested graphs as synced NamedRefs
- *(format)* warn when import clears an absent commit parent
- *(format)* add human/LLM-readable .gantz text format
- *(egui)* soften the diagnostic node glow
- *(std,egui)* log nodes report their emitting node; log view navigates
- *(egui)* highlight selected node spans in steel view with scroll-to
- *(egui)* highlight diagnostic nodes in the graph scene
- *(gui)* thread module artifact and diagnostics to frontends
- *(core)* structured node diagnostics from compile and runtime errors
- *(core)* run module as one steel program; map steel errors to nodes
- *(gui)* surface the compile config in the Graph Config pane
- *(compile)* introduce a compile config
- *(vm)* render the full error cause chain on compile failure
- *(app)* expose the delay node as a builtin
- consolidate frame! nodes into a single multi-source entrypoint ([#201](https://github.com/nannou-org/gantz/pull/201))
- close command palette on click outside
- add Branch node for conditional output activation
- add configurable output count to expr node
- set-based entrypoints with content-addressed IDs
- buffer comment text edits and flush on timeout or mouse activity
- Show on hover text for reset demo button
- add demo graph associations, reset, and UI polish
- add Ctrl+N shortcut to create new graph from scene
- paste nodes at mouse position from context menu
- add right-click context menus to graph scene nodes and background
- add right-click context menus to graph scene
- make node inspector labels act as selection controls
- Update to egui_graph 0.14
- deterministic GantzState HashMap serialization
- deterministic HashMap serialization via sorted keys

### Fixed

- *(egui)* breadcrumb labels, rename validation, nested-ref paste
- *(egui)* capture response payload type identity at emission
- *(egui)* align diagnostic glow to the frame's snapped edges; clip to pane
- *(egui)* match the glow corner radius to the node frame
- *(egui)* replace (not intersect) the glow painter's clip rect
- *(egui)* clip the diagnostic glow in scene-local coordinates
- *(egui)* open selection in new tab when focused head is unnamed
- Remove unnecessary max limit from node frames for expr/branch
- disable inspector UI for immutable nodes
- set comment node min scroll height to one row of the active font
- clip comment node text to frame instead of expanding
- disable inner vscroll on node inspector tables
- branch named nodes with independent commits
- create new commit on graph rename/branch

### Other

- *(egui)* drop redundant explicit intra-doc link targets
- *(egui)* inline the single-use index_path_node_mut
- *(egui)* drop the always-empty GraphScene path threading
- *(demo)* collapse the two apply-moves loops into one helper
- *(egui)* factor sync.rs clone-rewrite-recommit helpers
- *(egui)* collapse per-head views to a single egui_graph::View
- remove inline GraphNode; nested graphs are graph refs
- replace Lowerable trait alias with explicit, minimal bounds
- *(egui)* fix rustdoc warnings under -D warnings
- *(egui)* remove RON from export/import; clipboard uses .gantz text
- *(format)* extract gantz_format crate from gantz_egui
- *(format)* normalize structure into graph/commits/names tables
- *(egui)* rename response::Payload to DynResponse; fix doc link
- *(egui)* return emitted payloads from pane helpers
- *(egui)* replace Cmd queue with dynamic response channel
- *(egui)* extract shared frontend ops into gantz_egui::ops
- apply cargo fmt
- *(gui)* merge CompiledModule into Module; render errors separately
- Merge pull request #228 from mitchmindtree/feat/compile-config
- extract head_immutable to deduplicate immutability checks
- align eval/entry terminology in event and codegen pipeline
- rename eval-fn to entry-fn with shortened hash
- clean up entrypoint API and consolidate eval command pipeline
- lowercase hover text, mention Ctrl+N in "+" tooltip
