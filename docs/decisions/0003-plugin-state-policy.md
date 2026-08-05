# ADR-0003: Track plugin state opaquely; never interpret or port it

- **Status:** Accepted
- **Date:** 2026-08-05

## Context

A natural argument says plugins do not matter for version control: *an EQ or a compressor
ultimately just changes the wave, so if you have the audio you have the result.*

That is true — and it is exactly why it does not work.

If Wit versioned only rendered audio, plugin state would genuinely be irrelevant. But the
recipient would receive a photograph rather than a recipe: they could hear the sound and
could not change it. **The entire reason to send a project instead of a bounce is that
the receiving musician can still turn the knobs.** A collaboration tool that removes
editability has removed the reason it exists.

Plugin state is also not a minor field. Measured on real projects:

- **96.7%** of a real `.flp` is opaque variable-length plugin/channel state.
- In FL alone it appears in three forms with different properties: VST chunks inside a
  wrapper (9 KB for one instrument, containing an **absolute DLL path**), zlib streams
  for FL's own advanced synths (2 KB inflating to 65 KB), and plain fixed structs for
  simple devices.
- Logic's `AuCU` (plugin) record count moved 70 → 76 → 265 across saves — plugin activity
  dominates the change profile.

Meanwhile, porting plugin state *between* DAWs is not a hard problem so much as an
ill-posed one. A Serum patch is Serum's private binary format; it means nothing without
Serum instantiating it. Automation curve shapes, warp/stretch algorithms, sidechain
routing and bus topology differ per DAW by design.

## Decision

**Wit tracks plugin state as opaque content-addressed blobs. It never parses, interprets,
or converts it.**

1. **Content-address every plugin state blob.** Wit can then report *"the compressor on
   Drum Bus changed"* — which is the useful 90% — without knowing what changed inside.
2. **Inflate before hashing where the container is compressed.** FL's zlib-wrapped synth
   state re-deflates entirely on any parameter change, so hashing the compressed bytes
   makes every save look different. Inflate, then hash. (Same principle as ADR-0002's
   normalise-before-hashing rule.)
3. **Record requirements as first-class metadata.** Plugin name, vendor, version,
   unique ID, format (VST2/VST3/AU/CLAP), and architecture. Store the *reference*, never
   the plugin binary — redistributing it would be piracy.
4. **Never port plugin state across DAWs.** Out of scope, permanently.
5. **Preserve unmodelled bytes verbatim.** Anything Wit does not understand round-trips
   untouched. Wit must never be the reason a session fails to open.

### What the user sees

Instead of silence followed by a wall of blank inserts, Wit can say up front:

```
This version needs 2 plugins you do not have:
  · FabFilter Pro-Q 3   (VST3)  — used on 4 tracks
  · Serum               (VST3)  — used on 1 track
```

That is a real improvement over every current workflow, and it needs no knowledge of what
is inside the blob.

## Consequences

**Good**

- Simple, robust, and honest. Wit never corrupts plugin state because it never touches it.
- "Which plugins do I need?" is answerable before opening the project — a top-3 complaint
  in every collaboration workflow.
- No legal exposure from reverse-engineering plugin formats.

**Bad, and accepted**

- Diff granularity stops at the plugin boundary: *"the compressor changed"*, not *"ratio
  4:1 → 8:1"*. Deeper diffs are possible for the small subset of plain-struct devices, but
  that is a later refinement, not v1.
- Two people editing the *same* plugin on the *same* track is an unresolvable conflict
  requiring a human choice. This is correct behaviour, not a limitation to engineer away.
- A plugin whose state includes a regenerated UUID or timestamp will appear to change on
  every save. Logic demonstrably does this. Such fields must be identified per plugin and
  excluded, or Wit reports phantom changes.

## What would overturn this

- Wide adoption of a plugin-state standard that is introspectable across hosts. CLAP is
  the only plausible candidate and does not currently provide this.
- Evidence that users demand parameter-level plugin diffs strongly enough to justify
  per-plugin reverse engineering. Even then, scope it to a handful of popular plugins
  rather than as a general capability.

## Related

[ADR-0001](0001-version-the-recipe-not-the-render.md), [FORMATS.md](../FORMATS.md).
