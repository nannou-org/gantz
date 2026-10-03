# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/nannou-org/gantz/compare/gantz_cli-v0.1.0...gantz_cli-v0.2.0) - 2026-10-03

### Added

- [**breaking**] sync name metadata through the vault
- *(gantz_cli)* guard the vault directory against other versions
- *(gantz_collab)* [**breaking**] fail on an unreadable identity
- *(gantz_collab)* answer the version probe
- *(gantz_collab)* [**breaking**] exchange app versions in the vault handshake
- *(gantz_cli)* add the vault subcommand
- *(gantz_collab_sync)* sync a device with a vault
- *(gantz_collab)* [**breaking**] add the vault protocol

## [0.1.0](https://github.com/nannou-org/gantz/compare/gantz_cli-v0.0.1...gantz_cli-v0.1.0) - 2026-09-28

### Added

- *(gantz_cli)* add the run subcommand
- *(gantz_io)* add the main! node
- *(gantz_io)* add the gantz_io crate with the log node
- *(gantz_cli)* move the gantz CLI into a library crate

### Other

- *(gantz_egui)* move the bang and number nodes into gantz_egui
- *(gantz_core)* move the list node into gantz_core
