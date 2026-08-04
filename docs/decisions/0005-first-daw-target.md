# ADR-0005: Ableton Live is the first DAW target

- **Status:** Accepted
- **Date:** 2026-08-05

## Context

Wit must parse DAW project formats, and each one is significant work. The order matters:
the first target determines how good the first demo is, and therefore whether anyone
takes the project seriously.

The obvious criterion is market size. We think that is the wrong one to lead with.

Measured properties of each candidate:

| DAW | Format | Semantic diff today? | Stable IDs? |
|---|---|---|---|
| Ableton Live | gzip → XML | **yes, demonstrated** | **yes, verified over 10 saves** |
| Logic / GarageBand | chunked binary | object-level yes, parameter-level no | not verified |
| FL Studio | typed event stream | ≤ v24 yes; v25 obfuscated | n/a (order-dependent) |
| Studio One | ZIP + XML | likely | unverified |
| Cubase | MFC object stream | no parser for modern files | — |
| Pro Tools | obfuscated TLV | ~35% of blocks named | — |

Ableton is the only one where the full chain — parse → diff → merge → repack — has
actually been demonstrated end to end on real files.

## Decision

**Ableton Live first. Logic and GarageBand second (one parser serves both). FL Studio
third. Everything else later or never.**

Reasoning:

1. **It produces the best diff.** The first thing anyone sees is `wit diff`. On Ableton
   that is already `volume 0.794 → 0.525` and `clip moved 480 → 496`. On a format we can
   only partially parse, it would be "something changed" — which convinces nobody.
2. **IDs are verified stable**, which is the precondition for diff and merge. Nowhere else
   has this been confirmed.
3. **Merge is proven** — a clean 3-way merge of two disjoint edits produced a valid `.als`.
4. **Fast iteration.** The format is text under a gzip wrapper. A contributor can inspect
   it with `gunzip` and a text editor, which matters enormously for an open-source project
   that needs outside help.
5. **Live's own `Backup/` folder gives every user a ready-made version chain** to test
   against, from day one, with no setup.

Explicitly *not* the reason: market share. FL Studio and GarageBand likely have larger
install bases. Being excellent for one DAW beats being mediocre for five, and a
half-working diff would poison the project's credibility before it has any.

## Consequences

**Good**

- The v1 demo is genuinely impressive rather than merely plausible.
- Contributors can inspect the format with standard tools.
- Every Ableton user already has test material in `Backup/`.

**Bad, and accepted**

- Wit will be dismissed by non-Ableton users as "not for me" until Phase 3. Accepted:
  early credibility with one community beats shallow reach across several.
- Ableton's absolute sample paths are the worst of any format measured (777 references to
  another person's home directory in one real project). Sample resolution must be solved
  early rather than deferred.
- Ableton's XML has no published schema; parsing is empirical and will break on Live
  updates. Verbatim preservation of unmodelled fields is therefore mandatory, not optional.

## What would overturn this

- Discovering Ableton IDs are unstable across Live *versions* or across copy/paste. This
  is an open question in [EXPERIMENTS.md](../EXPERIMENTS.md) and would be serious.
- A Logic `ProjectData` payload map arriving from the community that makes Logic's diff
  as good as Ableton's — Logic's install base and its shared format with GarageBand would
  then make it the better first target.

## Related

[FORMATS.md](../FORMATS.md), [ROADMAP.md](../ROADMAP.md).
