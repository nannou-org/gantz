# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/nannou-org/gantz/compare/gantz_ca-v0.2.1...gantz_ca-v0.3.0) - 2026-07-02

### Added

- *(gantz_ca)* last-edit-wins merge resolution
- *(gantz_ca)* selectable merge conflict resolutions
- *(gantz_ca)* three-way graph merge
- *(gantz_ca)* node matching and structural graph diff
- *(gantz_ca)* commit history utilities
- *(gantz_ca)* add merge_parents to Commit for merge commits
- *(gantz_ca)* impl CaHash for [T; N]; relax content_addr to ?Sized
- *(ca)* add name-keyed graph descriptions to Registry

### Fixed

- *(ca)* content-address graphs by canonical node rank
- *(named-ref)* include `sync` in the content address so toggles persist

## [0.2.1](https://github.com/nannou-org/gantz/compare/gantz_ca-v0.2.0...gantz_ca-v0.2.1) - 2026-06-15

### Added

- *(format)* add human/LLM-readable .gantz text format
- add core entrypoint types and CaHash collection impls
- deterministic HashMap serialization via sorted keys

### Fixed

- *(ca)* clear an absent parent in Registry::add_commit
