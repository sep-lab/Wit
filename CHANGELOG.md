# Changelog

Notable changes to this project. Format based on [Keep a Changelog](https://keepachangelog.com/1.1.0/).

## [Unreleased]

Building the 0.0 pilot — no released artifact yet.

### Added
- **ADR-0006**: the first shipped product is a Logic/GarageBand-first, read-only
  comprehension app — no project-file write-path in this slice. See
  [docs/ROADMAP.md](docs/ROADMAP.md)'s "Now: the 0.0 pilot" section for the milestone plan.
- Rust workspace scaffolding (`crates/wit-model`, `crates/wit-diff`) and CI (`rust`,
  `licenses` jobs) — M0, [PR #13](https://github.com/sep-lab/Wit/pull/13)
- **M1** — Ableton `.als` parity port: `wit-model`, `wit-als`, `wit-diff`, `wit-cli`, a
  Rust port of the working Python semantic differ with golden byte-for-byte tests and
  three named bug fixes over the Python original (deterministic ordering, a rename
  bijection guard, and a device-settings-fingerprint check that catches a same-shape
  knob turn) — [PR #24](https://github.com/sep-lab/Wit/pull/24)
- **M2** — Logic/GarageBand `ProjectData` container walker: `wit-logic`, covering both
  `.logicx` and `.band` (same container, same magic bytes), with record framing, a tag
  census, and whitelist name/tempo extraction — [PR #25](https://github.com/sep-lab/Wit/pull/25)
- **M4** — Audio engine: `wit-audio`, covering `symphonia`-based decode
  (wav/aiff/caf/alac/flac/mp4/mp3/aac) with a typed, never-panics `DecodeError`; `i8`
  min/max peak computation for waveform rendering; `realfft`-based FFT cross-correlation
  alignment with a confidence gate (refuses below 1.5 rather than returning a
  low-confidence shift); and the null-diff verdict ladder ported verbatim from
  `experiments/null_diff.py` (−80/−40/−12 dB thresholds, relative-not-absolute
  reasoning) — [PR #TBD](https://github.com/sep-lab/Wit/pull/TBD)
- Research findings across Ableton `.als`, Logic `ProjectData`, GarageBand `.band` and
  FL Studio `.flp`, measured on real projects ([docs/EXPERIMENTS.md](docs/EXPERIMENTS.md))
- Working prototypes: Ableton semantic differ, CDC dedup harness, FLP parser, storage bench
- ADRs 0001–0006 covering the core design
- Open-source project scaffolding: license, contributing guide, code of conduct, security
  policy, issue/PR templates, CI

### Findings that shaped the design
- Byte-level versioning of rendered audio is not viable: a global gain change leaves
  **0.00%** of bytes reusable, and costs git a full extra copy
- Delta chains beat content-defined chunking **29×** on project-file history
  (~11 KB per Ableton save)
- Ableton track IDs are stable across saves, and a 3-way merge of disjoint edits is clean
- GarageBand shares Logic's container format — one parser serves both
- A real Logic library models from 21.9 GB of audio to **12.0 GB** *with* full version
  history — though most of that win is dedupe + FLAC, not versioning
