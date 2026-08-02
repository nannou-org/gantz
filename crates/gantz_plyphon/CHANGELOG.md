# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/nannou-org/gantz/compare/gantz_plyphon-v0.2.0...gantz_plyphon-v0.2.1) - 2026-08-02

### Other

- updated the following local packages: gantz_egui

## [0.2.0](https://github.com/nannou-org/gantz/compare/gantz_plyphon-v0.1.1...gantz_plyphon-v0.2.0) - 2026-07-23

### Added

- *(app,plyphon)* [**breaking**] register unit nodes, migrate ~sinosc/~lag to the table
- *(format,plyphon)* unit keyword sugar + datum-aware label stems
- *(plyphon,egui)* UnitNode inspector and graph UI
- *(plyphon)* unit descriptor table + the generic UnitNode
- *(plyphon)* [**breaking**] keyed multi-param state + ParamBinding keys
- [**breaking**] builtins are data
- *(plyphon)* DSP-graph discovery as a pure data walk
- *(ca,collab)* [**breaking**] concrete Registry, verified relays, RawGraph deleted
- [**breaking**] flip the app stack onto the concrete data registry
- canonical dehydration gate + skip-if-default serde sweep
- *(gantz_plyphon)* the ~playbuf sample-playback node
- *(gantz_plyphon)* thread BufferBinding through the compile pipeline
- *(gantz_plyphon)* AudioAsset canonical PCM + the dsp.buffer section
- *(plyphon)* style dsp signal edges in the graph view
- *(gantz_plyphon)* root-level dsp port classification
- *(bevy_gantz_plyphon)* surface per-head derive status via DspHead and a DSP pane
- *(gantz_plyphon)* add DeriveStatus and a readable derived-program description
- *(gantz_plyphon)* record per-port width/rate shapes during derivation
- *(gantz_plyphon)* add Display impls for DeriveError and FlattenError

### Other

- [**breaking**] purge typed-node CaHash
- [**breaking**] concrete Env, N-free plugins, app node trait retired
- *(plyphon,gantz)* graph-addr resolution and app-crate port
- *(gantz_egui)* named ExtPaneEntry + non_exhaustive ExtPaneCtx
- Merge pull request #332 from mitchmindtree/feat/hybrid-dsp-inputs

## [0.1.1](https://github.com/nannou-org/gantz/compare/gantz_plyphon-v0.1.0...gantz_plyphon-v0.1.1) - 2026-07-12

### Added

- *(gantz_plyphon)* add the ~sum node
- *(gantz_plyphon)* bridge every resolving edge across flatten boundaries
- *(gantz_plyphon)* sum multi-sourced inputs across instance boundaries
- *(gantz_plyphon)* sum multiple edges into a dsp input
- *(gantz_plyphon)* add a sum_signals channel-summing helper
- *(plyphon,bevy)* retry transient spawn failures next frame
- *(plyphon,bevy)* instancing is the default lowering for DSP refs
- *(bevy)* drive synths from the template pipeline
- *(plyphon)* staged template derivation + recursive instantiate
- *(plyphon)* flatten markers for instanced refs and root boundaries
- *(plyphon)* compile prep for instanced derivation
- *(dsp)* param-based bus/scope/fade wiring
- *(plyphon)* egui cargo feature gating the GUI impls
- *(core)* AsRefNode reference probe
- *(plyphon)* the plyphon base source
- *(bevy,plyphon)* DSP inline flag on refs
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

- *(gantz_plyphon)* unlink private Feed in instance module docs
- *(plyphon)* rename ui module to egui, one submodule per node
- *(plyphon)* consolidate egui impls into a ui module
- *(dsp)* cover nested-graph flattening
- *(gantz_plyphon)* rename the dsp socket type label to "signal"
- rebuild capped sample lists in one pass instead of push_back-per-element
- *(gantz_plyphon)* name dsp nodes after their ugens
- *(gantz_plyphon)* reuse gantz_core pull-eval order for synthdef ordering
