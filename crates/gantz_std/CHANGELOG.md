# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0](https://github.com/nannou-org/gantz/compare/gantz_std-v0.3.0...gantz_std-v0.4.0) - 2026-07-02

### Added

- *(gantz_format_derive)* derive macro for NodeTag
- declare node wire tags at their definition sites
- *(gantz_std)* own its node sugar via StdSugar
- *(number)* persist precision + push-eval via the content address
- *(number)* add min/max/precision/push-eval config to Number
- *(bang)* add a trigger input

### Fixed

- *(state,log)* register VM helpers once to stop per-recompile leak

### Other

- move NodeTag into dedicated gantz_nodetag crates
- drop typetag from the workspace
- *(number)* verify min/max clamp input-socket values at runtime

## [0.3.0](https://github.com/nannou-org/gantz/compare/gantz_std-v0.2.1...gantz_std-v0.3.0) - 2026-06-21

### Added

- *(base)* add pure-primitive node library with per-category demos

## [0.2.1](https://github.com/nannou-org/gantz/compare/gantz_std-v0.2.0...gantz_std-v0.2.1) - 2026-06-15

### Added

- *(std,egui)* log nodes report their emitting node; log view navigates

### Other

- apply cargo fmt
