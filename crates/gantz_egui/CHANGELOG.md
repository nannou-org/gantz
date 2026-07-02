# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/nannou-org/gantz/compare/gantz_egui-v0.4.0...gantz_egui-v0.5.0) - 2026-07-02

### Added

- *(gantz_format_derive)* derive macro for NodeTag
- declare node wire tags at their definition sites
- *(gantz_ca)* last-edit-wins merge resolution
- *(gantz_egui)* conflict resolution strategy selector on the merge row
- *(bevy_gantz_egui)* merge observer and demo arm
- *(gantz_egui)* merge row in the graph config pane
- *(gantz_egui)* merge candidates, previews and the merge_head op
- *(gantz_egui)* scroll the node inspector to the selected node
- *(gantz_egui)* log/trace view filter, options popup, column toggles
- *(gantz)* compose the per-crate node sugars via NodeSugar
- *(gantz_egui)* own its node sugar via EguiSugar
- *(gantz_egui)* node views for number/expr/branch; rename + revert ([#271](https://github.com/nannou-org/gantz/pull/271))
- *(gantz_egui)* full-bleed node views; plot fills its pane ([#271](https://github.com/nannou-org/gantz/pull/271))
- *(gantz_egui)* open a node's UI in a pane to monitor it (#271, part 2)
- *(gantz_egui)* add Select all, Cut and Duplicate command shortcuts
- *(gantz_egui)* add keymap + Settings -> Keybinds panel
- *(node)* add Hz rate mode to tick!
- *(egui)* collapse repeated log/trace entries with an occurrence count
- *(egui)* add center-view to the graph scene context menu
- *(egui)* persist graph camera as centre + zoom, not a rect
- *(egui)* add delete button to edge context menu
- *(number)* dialer UI for bounds, precision and push-eval toggle
- *(egui)* make Settings subtabs vertically scrollable
- *(egui)* expose egui_graph 0.16 snap, grid and align options
- *(plot)* state row in table, drop range labels
- *(plot)* lay min/max out as two columns on one range row
- *(plot)* square corners on no-margin, tidy range row, hover text
- *(plot)* refine inspector, frame and interaction per review
- *(plot)* refine plot node per review feedback
- *(plot)* add a configurable plot node to gantz_egui
- *(bevy_gantz)* drive recompiles from committed CA; commit at edit sites ([#159](https://github.com/nannou-org/gantz/pull/159))
- *(named-ref)* add sync/fork to the node context menu when outdated
- *(gantz_egui)* center the command palette over the graph scene
- add concise descriptions for all builtin and base nodes
- *(gantz_egui)* show node info in the command palette and graphs hover
- *(layout)* one-shot auto-layout & center-view with selection support
- *(palette)* place new nodes under the pointer and select them
- *(ops)* add commit_layout for layout-only commits
- *(comment)* auto-fit comment height to text content
- *(bang)* add a trigger input
- *(expr)* always expose at least one (trigger) input

### Fixed

- *(gantz_egui)* polish node-view panes per review ([#271](https://github.com/nannou-org/gantz/pull/271))
- *(plot)* stop leaking a Steel binding per recompile
- *(demo)* key demo associations by graph name instead of commit
- *(expr)* preserve outputs count when editing expression text
- *(gantz_egui)* don't pre-highlight a palette entry on open
- *(gantz_egui)* forward NodeUi::description through pointer/ref impls
- *(comment)* thin selection border with a wide invisible drag band
- *(named-ref)* include `sync` in the content address so toggles persist

### Other

- move NodeTag into dedicated gantz_nodetag crates
- drop typetag from the workspace
- port remaining typetag usages to impl_node_set_serde!
- *(gantz_egui)* side-by-side perf tiles at half height
- *(gantz_egui)* inlet/outlet socket editor as inspector rows
- *(gantz_egui)* grid layout for the Graph Config pane
- *(gantz_egui)* rename CommandPalette to NodePalette
- *(gantz_egui)* make node views first-class top-level tiles ([#271](https://github.com/nannou-org/gantz/pull/271))
- *(gantz_egui)* tidy keybinds panel; rebind New graph to Cmd/Ctrl+T
- *(gantz_egui)* grid layout + context-menu reset for keybinds panel
- represent graphs with plain petgraph::Graph, not StableGraph
- *(egui)* store tile trees under a stable key + add Clear egui memory
- *(widget)* add CheckboxEnabled, use it for range + precision
- *(inspector)* share bound_col for number + plot range rows
- *(number)* terse, hover-documented inspector layout
- house NodeUi response types in response.rs; rename ValidateCommitted
- *(gantz_egui)* NodeUi methods return changed-aware responses
- *(gantz_egui)* show palette descriptions inline instead of a side panel

## [0.4.0](https://github.com/nannou-org/gantz/compare/gantz_egui-v0.3.0...gantz_egui-v0.4.0) - 2026-06-21

### Added

- *(egui)* make the auto-layout socket-aware
- *(named-ref)* extend the reference-cycle guard to paste
- *(named-ref)* guard against reference cycles when adding a NamedRef
- *(egui)* add a "logs: open" inspector row to the log node
- *(egui)* "open logs" on the log node menu; logs hidden by default
- *(egui)* make Settings a toggleable, hideable pane
- *(egui)* move base/demo filters into a filter-options menu button
- *(egui)* closable Logs/Steel tray tabs; Logs open by default
- *(egui)* default the Settings tab to Global
- *(egui)* render Settings subtabs like egui_tiles tabs (no box)
- *(egui)* selected filter colour lerps between text and strong
- *(egui)* base/demo filters + right-click menu in the Graphs pane
- *(egui)* brighten the sidebar toggle on hover
- *(egui)* use a hamburger glyph for the sidebar toggle
- *(egui)* fixed sidebar/tray sizes, subtle arrow toggle, settings polish
- *(egui)* Settings tab with subtabs, tab-hide, reset-layout
- *(egui)* sidebar hamburger + tabbed pane layout
- *(format)* add inline-name export for base.gantz
- *(base)* add pure-primitive node library with per-category demos
- *(gui)* inlet/outlet hover docs

### Fixed

- *(egui)* size Expr/Branch editors to text so lines stop wrapping
- *(egui)* forward NodeUi::context_menu through the Box/&mut wrappers
- Use filter option button glyph that egui can render
- *(egui)* subtle close button on Logs/Steel tabs, like graph tabs
- *(egui)* make the sidebar open/close toggle smaller (24->18pt)
- *(egui)* wider default sidebar; breadcrumb beside toggle, not above
- *(egui)* stop sidebar width growing on reopen; reset pane visibility
- *(egui)* make sidebar/tray draggable again; perf panes off by default
- *(egui)* fixed-px sidebar width + fainter toggle arrow
- *(command-palette)* keep palette open when interacting with scrollbar
- *(base)* stamp base graphs with a fixed timestamp so reset keeps refs valid
- *(gui)* keep socket tooltip width adaptive
- *(gui)* drop description focus on commit

### Other

- *(demo)* render the demo on wgpu instead of glow
- *(deps)* bump bevy 0.18->0.19 and adopt the published egui 0.34 stack
- Disable base nodes by default, provide on_hover text for filter opts
- *(egui)* share a general Tab widget for all tabs
- *(gui)* store inlet/outlet docs on the nodes
- *(gui)* address socket-doc review feedback

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
