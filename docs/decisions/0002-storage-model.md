# ADR-0002: Storage — delta chains for projects, content addressing for audio

- **Status:** Accepted (supersedes an earlier CDC-only design)
- **Date:** 2026-08-05

## Context

Two kinds of content need storing, and they have opposite characteristics:

- **Project files** — ~0.5–1.5 MB, rewritten completely on every save, changing by a
  fraction of a percent each time. Hundreds of versions.
- **Audio** — 10–200 MB per file, written once, never modified. Hundreds of files,
  frequently identical across projects.

An earlier iteration of this design used content-defined chunking (CDC) for both. Direct
measurement showed that is wrong for project files.

| Material | CDC + zlib | zstd delta chain |
|---|---|---|
| 29 Ableton versions (283 MB logical) | 9.1 MB (~310 KB/save) | **314 KB (~11 KB/save)** |
| 10 Logic `ProjectData` versions | 2–87% reuse, mean 46% | **171 KB total (62×)** |

Delta chains are **29× better** on Ableton history and turn Logic from a weak case into a
strong one. The reason is structural: a DAW save produces many small scattered edits with
long verbatim runs between them. Copy/insert deltas encode that shape almost perfectly;
chunk boundaries get shredded by it.

Conversely, CDC is exactly right for audio, where the wins come from *whole-file* identity
across projects (measured: 24.5% of a real 26 GB library is byte-identical duplicates) and
from insert/shift tolerance (99.6% reuse on a time-shifted region).

## Decision

**Use both, matched to the content.**

### Object store

Content-addressed, BLAKE3, immutable objects. Same idea as git: an object's ID is the
hash of its content, so deduplication is a structural consequence rather than a feature.

### Project files → delta chains

Store a periodic full snapshot plus `zstd --patch-from` deltas against the previous
version. Checkpoint every N versions (N ≈ 50, to be tuned) so restoring version *k* never
costs more than N patch applications.

### Audio → content-addressed whole objects, chunked when large

- Hash the whole file first; identical files collapse immediately.
- For files above a threshold, additionally chunk with FastCDC so trims and shifts share
  content.
- Compress losslessly with FLAC where the format allows exact round-trip; otherwise
  store raw. **Measured 47.2% on a real library**, but this varies from 1.1% to 65%
  depending on material — never quote one number without saying what it was measured on.
- Renders are stored in a **separate, prunable cache namespace**, not in history
  (see ADR-0001).

### Rejected: audio-aware residual coding

An appealing idea is to delta two renders by aligning them and coding the residual
signal. **Do not ship this.** Measured across cases, the residual ranged from **4.7% to
105%** of simply storing the new render — on a trim it was *worse than doing nothing*.
Whether it wins depends on whether the edit is sparse or dense, which is not knowable in
advance.

The one instructive result: fitting the *operation* (a single gain parameter) collapsed
the residual from 68.0% to 12.6%. That is not an argument for residual coding — it is a
proof of the DAG thesis. Model the operation and you do not need the residual at all.

### Audio is stored as FLAC, never zlib-compressed WAV

Same losslessness, roughly **10× more bytes saved** on dense material (34.8% vs 3.6%
measured on a dense mastered stem). Record bit depth, sample rate and channel layout as
first-class metadata so the round-trip is verifiable.

### Normalise containers before hashing — highest-priority rule

Hash the *content*, never the container. `.als` is gzip: hashing the gzip stream makes
every save look different and destroys delta locality. Gunzip first, then hash and delta.
The same applies to Studio One's ZIP (extract entries first).

This mirrors git's clean/smudge filter boundary, and it is the single highest-leverage
implementation detail in this ADR.

### Dirty detection must be semantic, not hash-based

**Measured on Logic:** three consecutive saves had identical record censuses — no
semantic change whatsoever — yet differed by 97 and 140 bytes, because a plugin-state
UUID is regenerated on every save and ~20 float32 values are rewritten at a fixed stride.

So "the file hash changed" does **not** mean "the music changed". Wit must compare parsed
models to decide whether a commit is empty, or it will spam users with meaningless
versions. Ableton has the same property in a milder form (view state churns constantly).

## Consequences

**Good**

- Unlimited project history costs ~11–19 KB per save. Auto-commit on every save becomes
  affordable, which is what makes Wit useful single-player.
- Audio dedup is automatic and cross-project.
- Real libraries get smaller (measured: 21.9 GB of audio → 12.0 GB) while gaining full
  history — though the bulk of that is dedupe + FLAC rather than versioning.

**Bad, and accepted**

- Two storage paths is more complexity than one.
- Delta chains trade read cost for write cost; checkpointing is required and adds tuning.
- FLAC round-trip must be verified bit-exact per file, or Wit silently corrupts audio.
  This needs a test that re-decodes and compares hashes, always, with no fast path.
- Chained deltas mean corruption propagates. Store a hash per version and verify on
  restore.

## What would overturn this

- Delta chains failing on a DAW whose saves reorder content globally (FL Studio v25's
  scalar keystream is the known candidate). Fallback there: diff only variable-length
  events, or fall back to whole-object storage.
- CDC beating delta chains once real projects exceed some size we have not tested.

## Related

[ADR-0001](0001-version-the-recipe-not-the-render.md), [EXPERIMENTS.md](../EXPERIMENTS.md) §6.
