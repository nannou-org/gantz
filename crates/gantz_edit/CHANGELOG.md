# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/nannou-org/gantz/compare/gantz_edit-v0.1.0...gantz_edit-v0.2.0) - 2026-10-31

### Added

- carry node state through copy and paste
- keep node state across sessions
- *(gantz_edit)* rename named graphs across a registry

### Fixed

- *(bevy_gantz_egui)* commit layouts only after a layout edit

### Other

- [**breaking**] compose node kinds at run time
- [**breaking**] move the format, export and base modules to gantz_edit
- *(gantz_edit)* [**breaking**] make the node codec generic over the node type

## [0.1.0](https://github.com/nannou-org/gantz/compare/gantz_edit-v0.0.1...gantz_edit-v0.1.0) - 2026-10-17

### Other

- *(gantz_core)* [**breaking**] replace the inlet, outlet and delay bools with a role
- [**breaking**] move sections, ref resync and NamedRef into gantz_edit
- [**breaking**] add the gantz_edit crate with the view model
