# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
