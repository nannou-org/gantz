# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/nannou-org/gantz/compare/gantz_plyphon-v0.1.0...gantz_plyphon-v0.1.1) - 2026-07-09

### Added

- *(plyphon)* Config, Status and the DSP settings tab
- *(plyphon)* add builtins() list and node_dsp_of probe
- *(dsp)* flatten nested graphs before synthdef derivation
- *(dsp)* add nested-graph flattening pass
- *(dsp)* thread node paths through synthdef derivation
- *(gantz_plyphon)* ~bus synthdef boundaries with per-region synths
- *(gantz_plyphon)* per-ugen rate (ar/kr) on ~sinosc and ~lag
- *(bevy_gantz_plyphon)* crossfade synth replacement instead of a hard cut
- *(gantz_plyphon)* ~pack/~unpack channel-routing nodes
- *(gantz_plyphon)* multichannel signals as channel-group edges
- *(gantz_plyphon)* ~scopeout branches gating + configurable channels
- *(gantz_plyphon)* full-resolution ~tap capture via plyphon ScopeOut
- *(gantz_plyphon)* add the ~tap DSP->control monitor node
- *(gantz_plyphon)* DSP->control monitor return-path infra
- *(gantz_plyphon)* summarise DSP node state as queued-update count
- *(gantz_plyphon)* .gantz keyword sugar for ~sine/~out/~lag
- *(bevy_gantz_plyphon)* schedule control updates ahead of the audio clock
- *(gantz_plyphon)* queue timestamped control updates in param state
- *(gantz_plyphon)* DSP control inputs + a ~lag node
- *(gantz_plyphon)* DSP param values as node state + combined inspector row
- *(gantz_plyphon)* clickless param updates via plyphon Params + set_control
- *(gantz)* wire the DSP crates into the app
- *(gantz_plyphon)* synthdef compiler + ~sine/~out DSP nodes

### Fixed

- Address cargo doc warnings in dsp, compile mods
- *(docs)* Various grammatical cleanup in comments
- *(gantz_plyphon)* keep control-input feeds out of derived synthdefs
- *(gantz_plyphon)* fixed-width value dialer in param inspector rows
- *(gantz_core)* don't panic ordering a pull over a subset of inputs

### Other

- *(dsp)* cover nested-graph flattening
- *(gantz_plyphon)* rename the dsp socket type label to "signal"
- rebuild capped sample lists in one pass instead of push_back-per-element
- *(gantz_plyphon)* name dsp nodes after their ugens
- *(gantz_plyphon)* reuse gantz_core pull-eval order for synthdef ordering
