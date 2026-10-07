# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/nannou-org/gantz/compare/gantz_collab_sync-v0.2.0...gantz_collab_sync-v0.3.0) - 2026-10-07

### Added

- [**breaking**] log the devices that a vault refuses
- *(bevy_gantz_collab)* [**breaking**] link the app to a vault

## [0.2.0](https://github.com/nannou-org/gantz/compare/gantz_collab_sync-v0.1.1...gantz_collab_sync-v0.2.0) - 2026-10-03

### Added

- [**breaking**] sync name metadata through the vault
- *(gantz_collab_sync)* report the vault link status and failures
- *(gantz_collab)* answer the version probe
- *(gantz_collab)* [**breaking**] exchange app versions in the vault handshake
- *(gantz_collab_sync)* sync a device with a vault
- *(gantz_collab)* [**breaking**] add the vault protocol

### Other

- *(gantz_collab_sync)* track one piece of work per vault name

## [0.1.1](https://github.com/nannou-org/gantz/compare/gantz_collab_sync-v0.1.0...gantz_collab_sync-v0.1.1) - 2026-09-28

### Other

- updated the following local packages: gantz_ca, gantz_egui, gantz_collab

## [0.1.0](https://github.com/nannou-org/gantz/compare/gantz_collab_sync-v0.0.0...gantz_collab_sync-v0.1.0) - 2026-09-26

### Added

- *(gantz_collab_sync)* extract the host-agnostic session sync plane

### Fixed

- *(gantz_collab_sync)* feed a fetch response only to the fetch it answers
