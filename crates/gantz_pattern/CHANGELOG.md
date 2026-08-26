# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/nannou-org/gantz/compare/gantz_pattern-v0.0.2...gantz_pattern-v0.1.0) - 2026-08-26

### Added

- *(gantz_pattern)* settle-based pm error feedback, rename psecs
- *(gantz)* register the pm node and use it in the demo
- *(gantz_pattern)* add the pm node, parsing at graph compile time
- *(gantz_pattern)* parse mini-notation in rust, emitting combinators
- *(gantz_pattern)* represent events as a transparent struct
- *(gantz_pattern)* add the pm node and a mini-notation demo
- *(gantz_pattern)* add runtime mini-notation via pat/m
- *(gantz_plyphon)* accept timestamped batches on control inputs
- *(gantz)* wire the pattern domain base file and steel module
- *(gantz_pattern)* add the query windower and delivery helpers
- *(gantz_pattern)* add euclidean rhythms via bjorklund
- *(gantz_pattern)* add joins, the apply family and map/filter/merge
- *(gantz_pattern)* add rates, cats, shift and fit-span
- *(gantz_pattern)* add constructors and sorted query
- *(gantz_pattern)* add span algebra and event representation

### Fixed

- *(gantz_pattern)* size the pm editor to its notation
- *(gantz_pattern)* stop tick! clobbering the demo's cps dial
- *(gantz_pattern)* silence partial evals throughout the module
- *(gantz_pattern)* reset the query window on jumps beyond a cap

### Other

- *(gantz_pattern)* rename the pm node to pmini
- *(gantz_pattern)* make the pm node pure, error on bad notation
- *(gantz_pattern)* simplify the demo to the audio leg
- *(gantz_pattern)* p-prefix the euclid nodes too
- *(gantz_pattern)* address review on naming and comments
- *(gantz_pattern)* license under GPL-3.0-or-later
