# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/nannou-org/gantz/compare/gantz_core-v0.2.0...gantz_core-v0.3.0) - 2026-06-15

### Added

- *(gui)* thread module artifact and diagnostics to frontends
- *(core)* structured node diagnostics from compile and runtime errors
- *(core)* run module as one steel program; map steel errors to nodes
- *(core)* source map over emitted steel module text
- *(compile)* introduce a compile config
- *(vm)* render the full error cause chain on compile failure
- *(compile)* [**breaking**] cut over to the IR pipeline; delete the flow codegen
- *(compile)* delay-cell feedback (pd-style cross-evaluation cycles)
- *(compile)* call-based nested graphs in the IR pipeline (Phase 2)
- *(compile)* lower single-level graphs through the IR (module_v2)
- *(compile)* add the join-point IR and its Steel emitter
- branch-aware push-through-outlet propagation
- implement Node::branches for GraphNode (nested-graph branching)
- consolidate frame! nodes into a single multi-source entrypoint ([#201](https://github.com/nannou-org/gantz/pull/201))
- implement nested entrypoint outlet propagation ([#208](https://github.com/nannou-org/gantz/pull/208))
- add $?var syntax for optional inputs in Expr and Branch nodes
- emit list bindings for unconditional multi-edge inputs
- add Branch node for conditional output activation
- add configurable output count to expr node
- support nested entrypoints in compilation pipeline
- set-based entrypoints with content-addressed IDs
- add core entrypoint types and CaHash collection impls
- add typed visitor support for downcasting during node traversal
- add demo graph associations, reset, and UI polish

### Fixed

- *(tests)* adapt the config test to the Compiled module api
- *(compile)* validate IR in every build; prune dead RoseTree helpers
- propagate active-input-set into nested graph compilation
- hoist root-level outlet vars so subgraphs open as roots
- order multi-root flow graphs so a branch-join's predecessors precede it
- recurse into nested graphs when discovering default entrypoints
- handle branch nodes with dead branches in codegen
- preserve distinct branch edges when outputs share a target
- skip destructuring for nodes with no connected outputs
- use `list` over `values` for multi-output nested graphs
- Address unused code warnings in rosetree
- Amend entrypoint docs to remove same-level source assumption
- Use correct Display fmt for thiserror errors

### Other

- *(core)* drop dead graph_partial_eq, refresh nested-test wording
- remove inline GraphNode; nested graphs are graph refs
- apply cargo fmt
- *(gui)* merge CompiledModule into Module; render errors separately
- *(core)* consolidate emitted-name format/parse in compile::names
- *(compile)* note IR validation runs in every build
- *(compile)* [**breaking**] analyse outlet activation over the IR; delete flow.rs
- drop the Node::graph marker; graph nodes compile uniformly
- *(compile)* rename V2 to ModuleBuilder
- pin Steel target semantics the IR emitter relies on
- fold reduced-inner-conf collection into one top-down builder
- drop intra-doc link from public nested_expr to private graph_branches
- apply cargo fmt to nested-branching changes
- port additional nested-branching edge cases
- replace outlet-activation bool with an OutletActivity enum
- build flow graphs after child recursion in build_flow_tree
- add branch codegen edge-case tests
- unify input bindings with before-target approach
- replace DFS reconvergence with post-dominator tree
- handle branch-join reconvergence via phi variables in codegen
- align eval/entry terminology in event and codegen pipeline
- rename eval-fn to entry-fn with shortened hash
- guard empty EvalSource path, add nested outlet propagation test
- clean up entrypoint API and consolidate eval command pipeline
- add multi-source entrypoint and naming consistency tests
