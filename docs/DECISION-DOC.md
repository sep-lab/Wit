# Decision document: is version control for music projects feasible?

**Date:** 2026-08-05 · **Status:** Accepted — proceed · **Scope:** technical feasibility

This is the document a new contributor should read to understand *why the project exists*
and *what has already been settled*. Detailed evidence is in
[EXPERIMENTS.md](EXPERIMENTS.md); individual decisions are in [decisions/](decisions/).

---

## Verdict

**Feasible, with one large caveat and one hard constraint.**

- **Feasible:** DAW project files are small, structured, have stable identifiers, and
  change by a fraction of a percent per save. Semantic diff and three-way merge have both
  been demonstrated on real files. Full version history costs ~11 KB per save.
- **Large caveat:** it requires reverse-engineering proprietary formats, per DAW, ongoing.
  There is no shortcut and no standard to lean on.
- **Hard constraint:** rendered audio **cannot** be versioned at the byte level. This is
  not an engineering gap; it is a property of the data. The architecture must route
  around it, and everything else follows from that.

---

## The question we actually had to answer

The proposal was "git for waves": treat audio as binary and let a content-addressed store
handle it, the way git handles files.

We tested the premise directly rather than assuming it, on a real 19.6 MB stem, simulating
three things producers genuinely do:

| Producer action | Bytes reusable | git incremental cost |
|---|---|---|
| Moved a region in time (+250 ms) | 99.59% | — |
| Re-rendered after editing one 5-second section | 92.98% | +1.1 MB |
| Re-rendered after an EQ change on the track | **0.00%** | **+16.8 MB (full copy)** |

**Row three is the whole finding.** Changing one EQ band alters every sample value in the
file. Nothing is shared, so nothing can be deduplicated or delta-encoded — not by git, not
by content-defined chunking, not by any future algorithm. Perceptual similarity is not
byte similarity.

To be fair to git, whose delta engine is better than its reputation: it handles the
*localized* case well (+1.1 MB for a 93%-reusable change) and dedups identical files
perfectly. Its failure here is narrow and specific — **global DSP** — and that case happens
every day.

### The resolution

A music project is already a program: **immutable source recordings plus a graph of
non-destructive operations** that renders to audio. That structure is exactly what the
DAW project file contains. It is small, structured, diffable, and mergeable.

So: **version the recipe, not the render.** Renders are build artifacts — you do not
commit `node_modules`. Confirming the point, 52% of one real Ableton project's audio
folder was `Freeze/`, `Consolidate/` and `Crop/` output: derived data, regenerable, not
history. ([ADR-0001](decisions/0001-version-the-recipe-not-the-render.md))

---

## What made us confident

Four results, each measured on real commercial-grade projects.

**1. Saves are tiny.** Consecutive Ableton autosaves differ by **0.07–0.25%** of a
232,000-line file. One 202-line diff was entirely a sample rename; one 170-line diff was
entirely scroll position and zoom.

**2. Identifiers are stable.** Ableton track IDs were identical across all 10 saves. This
is the precondition for everything — without it you cannot distinguish "this track
changed" from "one track deleted, another added".

**3. Edits are local, so merge is real.** A median of **3 of 28 tracks** change per save.
A three-way merge of two disjoint edits produced a clean, valid `.als` containing both.

**4. History is nearly free.** 29 versions of a real project: 283 MB logical XML →
**314 KB** stored, ~11 KB per save. And on a real 26 GB, 30-project library — of which
**24.5% is byte-identical duplicate audio** created by `Save As` branching — the modelled
result is **8.1 GB with full history**.

That last number is the strongest single argument: **Wit is smaller than the status quo
while adding history that does not exist today.**

---

## What we decided

| # | Decision | Rationale |
|---|---|---|
| [0001](decisions/0001-version-the-recipe-not-the-render.md) | Version the recipe, not the render | 0% byte reuse on global re-render |
| [0002](decisions/0002-storage-model.md) | Delta chains for projects; content addressing for audio | Delta chains beat CDC **29×** on project history |
| [0003](decisions/0003-plugin-state-policy.md) | Track plugin state opaquely; never interpret or port | 92% of an `.flp` is opaque state; cross-DAW porting is ill-posed |
| [0004](decisions/0004-implementation-stack.md) | Rust core; Python for research | GB-scale hashing, single binary, safe binary parsing |
| [0005](decisions/0005-first-daw-target.md) | Ableton first | Only format where the full chain is demonstrated |

Two decisions surprised us enough to be worth flagging:

**Storage.** We began assuming content-defined chunking was correct for everything.
Measurement showed delta chains are 29× better for project files (11 KB vs 310 KB per
save), because DAW saves produce many small scattered edits with long verbatim runs
between them — the shape copy/insert deltas encode almost perfectly and chunk boundaries
handle badly. CDC remains right for audio. We use both.

**Normalisation matters more than any algorithm.** `.als` is gzip. Hashing the gzip stream
makes every save look unrelated; gunzipping first cut a repo **7×** and cut per-save cost
**30×**. Git's clean/smudge filter boundary is the pattern; this is the highest-leverage
implementation detail in the design.

---

## Risks we are accepting

| Risk | Severity | Mitigation |
|---|---|---|
| **Formats are undocumented and shift under us** | High | Preserve unmodelled bytes verbatim; Wit must never be why a session fails to open |
| **A Wit-written file might not open in the DAW** | High | Untested today. Hard release gate for Phase 1. |
| **Hash ≠ change** | Medium | Logic regenerates a plugin UUID and rewrites ~20 float32s *every save* with no semantic change. Dirty detection must compare parsed models, or Wit spams empty commits. |
| **Plugin state is unmergeable** | Medium | Two people editing one plugin is a human decision, not an algorithm. Correct behaviour, not a gap. |
| **FL Studio v25 obfuscation** | Medium | Target ≤ v24 first; treat v25 as an open problem. |
| **Legal exposure on Pro Tools** | Low | Do not ship `.ptx` deobfuscation. Storage-level support needs no decryption. |
| **Sample licensing** | Low | Wit does not host third-party audio. |

---

## What is still unknown

Honest list — none of these are settled, and the first two are the ones that matter:

1. **Does a Wit-produced project open in the DAW?** No DAW was launched during this work.
   Blocking for any release.
2. **Are DAW renders deterministic?** If bouncing twice is not bit-identical, "did the
   audio change" cannot be answered by hashing renders — which affects cache invalidation
   and any future audible-diff feature.
3. **Are Ableton IDs stable across Live *versions*, and across duplicate/copy-paste?**
   Only within-version stability was verified.
4. **Logic `ProjectData` payload schemas.** The container is decoded; the payloads are not.
5. **Breadth.** Every number here comes from a handful of projects on one machine. This is
   the single biggest weakness of this document, and the easiest one for contributors to
   fix.

---

## What would change the verdict

- Ableton IDs turning out to be unstable across versions — that would undermine diff and
  merge, and would force a rethink.
- Merged files being rejected by DAWs in ways that cannot be fixed.
- Evidence that users only want to version *bounces* and do not care about editability —
  which would make this a much simpler and much less interesting product.

## Out of scope

Cross-DAW project conversion, real-time collaborative editing, hosting third-party
samples, Pro Tools deobfuscation, and being a DAW. See [ROADMAP.md](ROADMAP.md).
