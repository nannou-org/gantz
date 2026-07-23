# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/nannou-org/gantz/compare/bevy_gantz_plyphon-v0.1.1...bevy_gantz_plyphon-v0.2.0) - 2026-07-23

### Added

- *(app,plyphon)* [**breaking**] register unit nodes, migrate ~sinosc/~lag to the table
- *(plyphon)* [**breaking**] keyed multi-param state + ParamBinding keys
- [**breaking**] the working graph is data
- [**breaking**] builtins are data
- *(plyphon)* DSP-graph discovery as a pure data walk
- *(ca,collab)* [**breaking**] concrete Registry, verified relays, RawGraph deleted
- [**breaking**] flip the app stack onto the concrete data registry
- *(bevy_gantz_plyphon)* resident buffer table + refcount for ~playbuf
- *(gantz_plyphon)* thread BufferBinding through the compile pipeline
- *(plyphon)* style dsp signal edges in the graph view
- *(bevy_gantz_plyphon)* persist derived port shapes on DspHead
- *(bevy_gantz_egui)* EdgeStyles provider resource
- *(bevy_gantz_plyphon)* surface per-head derive status via DspHead and a DSP pane
- *(gantz_plyphon)* record per-port width/rate shapes during derivation

### Other

- *(bevy_gantz_plyphon)* audible unit-chain e2e with keyed control
- [**breaking**] purge typed-node CaHash
- [**breaking**] concrete Env, N-free plugins, app node trait retired
- *(gantz,bevy_gantz_plyphon)* end-to-end ~playbuf playback + reachability
- *(plyphon,gantz)* graph-addr resolution and app-crate port
- Merge pull request #332 from mitchmindtree/feat/hybrid-dsp-inputs

## [0.1.1](https://github.com/nannou-org/gantz/compare/bevy_gantz_plyphon-v0.1.0...bevy_gantz_plyphon-v0.1.1) - 2026-07-12

### Added

- *(plyphon,bevy)* retry transient spawn failures next frame
- *(plyphon,bevy)* instancing is the default lowering for DSP refs
- *(bevy)* drive synths from the template pipeline
- *(plyphon)* flatten markers for instanced refs and root boundaries
- *(plyphon)* compile prep for instanced derivation
- *(dsp)* driver refcounted shared defs (install once, spawn many)
- *(dsp)* param-based bus/scope/fade wiring
- *(plyphon)* egui cargo feature gating the GUI impls
- *(core)* AsRefNode reference probe
- *(plyphon)* the plyphon base source
- *(bevy,plyphon)* DSP inline flag on refs
- *(compile)* debug-log each compile step with its duration
- *(dsp)* flatten nested graphs before synthdef derivation
- *(web)* AudioWorklet audio for the gantz website
- *(gantz_plyphon)* ~bus synthdef boundaries with per-region synths
- *(bevy_gantz_plyphon)* crossfade synth replacement instead of a hard cut
- *(gantz_plyphon)* multichannel signals as channel-group edges
- *(gantz_plyphon)* ~scopeout branches gating + configurable channels
- *(gantz_plyphon)* full-resolution ~tap capture via plyphon ScopeOut
- *(gantz_plyphon)* DSP->control monitor return-path infra
- Settings -> Audio tab (status + scheduling lead + mute)
- *(bevy_gantz_plyphon)* hook to register custom plyphon units
- *(bevy_gantz_plyphon)* schedule control updates ahead of the audio clock
- *(gantz_plyphon)* DSP param values as node state + combined inspector row
- *(gantz_plyphon)* clickless param updates via plyphon Params + set_control
- *(bevy_gantz_plyphon)* cpal audio runtime driving derived synths

### Fixed

- *(plyphon)* construct the DSP engine in Plugin::finish
- *(docs)* Various grammatical cleanup in comments
- *(bevy_gantz_plyphon)* free-run the engine clock on the web audio thread
- *(bevy_gantz_plyphon)* drive the audio clock with web_time::Instant

### Other

- *(bevy,gantz)* instancing runtime coverage
- *(bevy)* parts data model + BusKey-keyed bus allocation
- *(bevy)* move DSP settings onto the domain seam
- *(dsp)* rename the Audio* abstraction to Dsp*
- *(web)* drop the legacy web build, promote AudioWorklet to canonical
- *(gantz_plyphon)* name dsp nodes after their ugens
- *(bevy_gantz_plyphon)* headless custom-unit example
