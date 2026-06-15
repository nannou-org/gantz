# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/nannou-org/gantz/compare/gantz_base-v0.2.0...gantz_base-v0.2.1) - 2026-06-15

### Added

- *(format)* add human/LLM-readable .gantz text format
- deterministic HashMap serialization via sorted keys

### Fixed

- *(ca)* clear an absent parent in Registry::add_commit

### Other

- *(format)* extract gantz_format crate from gantz_egui
- *(format)* normalize structure into graph/commits/names tables
