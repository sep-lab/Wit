# ADR-0001: Version the recipe, not the render

- **Status:** Accepted
- **Date:** 2026-08-05

## Context

The intuitive design for "git for waves" is to version audio files directly: audio is
already 0s and 1s, so let a content-addressed store diff the bytes the way git does.

We tested it on a real 19.6 MB 24-bit/48 kHz stem, simulating three things producers
actually do, and measuring how much of the new version could be reused from the old one.

| Producer action | Bytes reusable (CDC) | git incremental cost |
|---|---|---|
| Moved the region in time (+250 ms) | 99.59% | — |
| Re-rendered after editing one 5-second section | 92.98% | +1.1 MB |
| Re-rendered after an EQ change across the track | **0.00%** | **+16.8 MB (full copy)** |

The third row is decisive. Changing one EQ band alters *every sample value* in the file.
Nothing is shared, so nothing can be deduplicated or delta-encoded — not by git, not by
content-defined chunking, not by any future algorithm. Perceptual similarity is not byte
similarity.

Compression does not rescue it either: zlib (which git uses) keeps 82.6% of PCM audio;
even FLAC keeps 49.9%.

Meanwhile the *project file* — which fully describes how to produce that audio — behaves
beautifully. Consecutive Ableton saves differ by 0.07–0.25%, and 29 versions of a real
project store in **314 KB total, ~11 KB per save**.

## Decision

**Wit versions the recipe: immutable source audio plus the graph of non-destructive
operations that produces the result. Rendered audio is treated as a build artifact.**

Concretely, three classes of content:

1. **Source audio** — recordings and imported samples. Immutable once captured;
   content-addressed and stored exactly once, forever. Deduplicates ~100% across versions
   *and* across projects.
2. **Project structure** — tracks, clips, positions, automation, mixer state, device
   chains. Small, structured, diffable, mergeable. Stored as delta chains.
3. **Renders** — bounces, freezes, consolidations, stem exports. **Derived.** Rebuildable
   from 1 + 2. Cacheable, not history.

That third category is not a rounding error: in the Ableton project measured, **52% of
the audio folder** was `Freeze/`, `Consolidate/` and `Crop/` output — regenerable data.

## Consequences

**Good**

- Version history becomes nearly free (~11 KB/save), so Wit can commit aggressively —
  every save, automatically — instead of asking users to curate.
- Diffs become musically meaningful, because the thing being diffed is musical structure.
- Merge becomes possible at all: you cannot merge two renders, but you can merge two
  edit graphs.
- Storage shrinks on real libraries even while gaining history (measured: 21.9 GB of
  audio → 12.0 GB, 1.8×). Note most of that is dedupe + FLAC, not versioning.

**Bad, and accepted**

- Wit must **parse DAW project formats**. This is the hard part of the project and the
  main source of ongoing work. There is no shortcut.
- A commit is only as faithful as the parser. Unmodelled fields must be preserved
  verbatim, never dropped.
- "Play me v3" may require the DAW to re-render. Wit mitigates this by caching the last
  render per commit, but the cache is explicitly not the source of truth.
- Wit is DAW-specific by nature. It cannot be a generic file syncer, and it degrades to
  plain content-addressed storage for formats it does not understand.

## What would overturn this

- A demonstration that some audio-aware delta (e.g. align-then-code-the-residual) makes
  global re-renders cheap to store. We consider this unlikely — a filter changes phase
  and amplitude everywhere — but it has not been rigorously disproven, only measured as
  0% for byte-level methods.
- Evidence that users overwhelmingly want to version *bounces* and do not care about
  editability. That would make Wit a different (and much simpler) product.

## Related

[ADR-0002](0002-storage-model.md), [ADR-0003](0003-plugin-state-policy.md),
[EXPERIMENTS.md](../EXPERIMENTS.md) §6 and §7.
