# Changelog

Notable changes to this project. Format based on [Keep a Changelog](https://keepachangelog.com/1.1.0/).

## [Unreleased]

Design phase — no released artifact yet.

### Added
- Research findings across Ableton `.als`, Logic `ProjectData`, GarageBand `.band` and
  FL Studio `.flp`, measured on real projects ([docs/EXPERIMENTS.md](docs/EXPERIMENTS.md))
- Working prototypes: Ableton semantic differ, CDC dedup harness, FLP parser, storage bench
- ADRs 0001–0005 covering the core design
- Open-source project scaffolding: license, contributing guide, code of conduct, security
  policy, issue/PR templates, CI

### Findings that shaped the design
- Byte-level versioning of rendered audio is not viable: a global EQ change leaves
  **0.00%** of bytes reusable, and costs git a full extra copy
- Delta chains beat content-defined chunking **29×** on project-file history
  (~11 KB per Ableton save)
- Ableton track IDs are stable across saves, and a 3-way merge of disjoint edits is clean
- GarageBand shares Logic's container format — one parser serves both
- A real Logic library models from 21.9 GB of audio to **12.0 GB** *with* full version
  history — though most of that win is dedupe + FLAC, not versioning
