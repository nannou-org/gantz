# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/nannou-org/gantz/compare/bevy_gantz_collab-v0.1.0...bevy_gantz_collab-v0.1.1) - 2026-08-26

### Other

- updated the following local packages: gantz_core, bevy_gantz, gantz_egui, bevy_gantz_egui

## [0.1.0](https://github.com/nannou-org/gantz/compare/bevy_gantz_collab-v0.0.1...bevy_gantz_collab-v0.1.0) - 2026-07-23

### Added

- *(gantz_egui)* default the action send rate to ~16ms
- *(collab)* live peer pointers over shared graphs
- *(gantz_egui,bevy_gantz_collab)* configurable action send rate
- *(collab)* overlay sync progress until the join snapshot arrives
- *(bevy_gantz_collab)* remote action application
- *(bevy_gantz_collab)* action capture + broadcast
- *(gantz_collab)* GossipMsg::Action envelope
- *(bevy_gantz_egui,bevy_gantz_collab)* session undo/redo mints revert commits
- *(collab)* relay visibility and a custom relay setting
- *(collab)* joining opens the session tab immediately with progress
- *(collab)* sync node layouts between session peers
- *(gantz_egui)* collab UI - session row, Settings subtab, payloads
- *(bevy_gantz_collab)* session plugin bridging the collab runtime

### Fixed

- *(bevy_gantz_collab,gantz_egui)* batch rate-limited writes, replay all
- *(collab)* move mod and use decls to start of mod
- *(bevy_gantz_collab)* keep a fused push-eval when a newer write lands
- *(bevy_gantz_egui,bevy_gantz_collab)* announce sessions after view persistence
- *(gantz_egui,bevy_gantz_egui,bevy_gantz_collab)* seed merge-commit views at mint

### Other

- *(bevy_gantz_collab)* split lib.rs into ui, session and sync modules
- *(bevy_gantz_collab)* generic mark_dirty; SyncCtx system-param bundle
- *(bevy_gantz_egui)* shared finish for locally-minted merge commits
