# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0](https://github.com/nannou-org/gantz/compare/bevy_gantz-v0.3.1...bevy_gantz-v0.4.0) - 2026-07-02

### Added

- *(storage)* add BatchWriter, a buffering Save impl
- *(persist)* log persist duration and on-disk counts
- *(bevy_gantz)* drive recompiles from committed CA; commit at edit sites ([#159](https://github.com/nannou-org/gantz/pull/159))
- *(bevy_gantz)* persist and apply graph descriptions
- *(head)* preserve VM state across same-graph navigation

### Other

- *(node)* rename frame! to update!
- *(persist)* stagger egui memory onto a separate debounce
- *(persist)* persist registry incrementally
- house NodeUi response types in response.rs; rename ValidateCommitted

## [0.3.1](https://github.com/nannou-org/gantz/compare/bevy_gantz-v0.3.0...bevy_gantz-v0.3.1) - 2026-06-21

### Other

- *(deps)* bump bevy 0.18->0.19 and adopt the published egui 0.34 stack

## [0.3.0](https://github.com/nannou-org/gantz/compare/bevy_gantz-v0.2.0...bevy_gantz-v0.3.0) - 2026-06-15

### Added

- *(gui)* thread module artifact and diagnostics to frontends
- *(core)* run module as one steel program; map steel errors to nodes
- *(gui)* surface the compile config in the Graph Config pane
- *(compile)* introduce a compile config
- *(vm)* render the full error cause chain on compile failure
- consolidate frame! nodes into a single multi-source entrypoint ([#201](https://github.com/nannou-org/gantz/pull/201))
- set-based entrypoints with content-addressed IDs

### Fixed

- hoist root-level outlet vars so subgraphs open as roots
- branch named nodes with independent commits
- create new commit on graph rename/branch

### Other

- *(bevy)* input-addressed VM sync replaces scattered recompile paths
- apply cargo fmt
- *(gui)* merge CompiledModule into Module; render errors separately
- align eval/entry terminology in event and codegen pipeline
- rename eval-fn to entry-fn with shortened hash
- clean up entrypoint API and consolidate eval command pipeline
