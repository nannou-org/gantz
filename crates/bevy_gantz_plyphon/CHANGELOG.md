# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/nannou-org/gantz/compare/bevy_gantz_plyphon-v0.1.0...bevy_gantz_plyphon-v0.1.1) - 2026-07-09

### Added

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

- *(docs)* Various grammatical cleanup in comments
- *(bevy_gantz_plyphon)* free-run the engine clock on the web audio thread
- *(bevy_gantz_plyphon)* drive the audio clock with web_time::Instant

### Other

- *(bevy)* move DSP settings onto the domain seam
- *(dsp)* rename the Audio* abstraction to Dsp*
- *(web)* drop the legacy web build, promote AudioWorklet to canonical
- *(gantz_plyphon)* name dsp nodes after their ugens
- *(bevy_gantz_plyphon)* headless custom-unit example
