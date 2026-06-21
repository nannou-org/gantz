# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/nannou-org/gantz/compare/gantz_format-v0.2.0...gantz_format-v0.2.1) - 2026-06-21

### Added

- *(format)* add inline-name export for base.gantz
- *(format)* round-trip inlet/outlet socket docs
- *(base)* add pure-primitive node library with per-category demos

### Other

- *(format)* drop private intra-doc link in to_string_named

## [0.2.0](https://github.com/nannou-org/gantz/compare/gantz_format-v0.0.1...gantz_format-v0.2.0) - 2026-06-15

### Added

- *(format)* add Datum serde codec over reader-valid Steel datums

### Fixed

- *(egui)* breadcrumb labels, rename validation, nested-ref paste
- top-level gantz_format description header

### Other

- remove inline GraphNode; nested graphs are graph refs
- *(format)* cohesion pass - Datum accessor methods + shared helpers
- replace Lowerable trait alias with explicit, minimal bounds
- *(format)* cover Datum codec edge cases
- *(format)* Datum node bridge + pluggable Sugar, drop serde_json
- *(format)* extract gantz_format crate from gantz_egui
