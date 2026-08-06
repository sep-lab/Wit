# ADR-0006: The first product is a Logic-first, read-only comprehension surface

- **Status:** Accepted
- **Date:** 2026-08-06

## Context

Phase 0 (research) is complete: measured diff/dedup/ID-stability facts, five ADRs, a
prior-art graveyard, five Python prototypes and a shell harness, 266 tests, and CI that machine-enforces the
repo's own honesty rules. What does not exist is any product — no Rust, no `wit` binary,
no interface a musician can touch. Engagement is nil.

Two things changed since ADR-0005 fixed Ableton as the first *parser* target:

1. **The first cohort is known, and it is not Ableton users.** The intended first users
   — Sepehr's friends — are Logic Pro producers; Ableton is nice-to-have. ADR-0005 chose
   Ableton on parser readiness (semantic diff demonstrated, IDs verified stable) without
   an identified cohort to hand it to. A parser with no one to use it is not a product.
2. **The adoption evidence points away from deliberate-act tooling.** *Measured*, across
   30 real Logic projects: Logic ships free one-click branching (Alternatives), used
   **zero** times, while 93 automatic backups accumulated unrequested. Deliberate-act
   tooling (branch, commit, merge) is declined; passive, ambient tooling is used without
   being asked for. This makes **comprehension** ("what changed?") the
   untested-not-refuted hypothesis, not branch-and-merge.

Sepehr resolved the resulting tension directly: build the **musician-usable read-only
comprehension slice** — passive auto-history, readable "what changed", sharing — with
**no project-file write-path**. Because it never writes to a project, ROADMAP blocker #1
("no Wit-produced file has ever been opened in a DAW") does not gate this slice; restore
is reveal-or-copy, never an edit. Because it is read-only, it can target Logic immediately
without waiting on ADR-0005's Ableton-first parser roadmap.

**New format evidence, probed 2026-08-06** (measured on copies of real local
`.logicx`/`.band` packages, originals untouched — the probe script is a throwaway,
not product code, and is not part of this repo):

- The 24-byte root header, 36-byte record framing, and `qeSM`/`AuRg`/`AuFl` payload
  reads documented in [FORMATS.md](../FORMATS.md) walk to clean EOF on every real file
  tested, across three version words (`d009`, `d109`, and GarageBand's `c509` — more
  variance than FORMATS.md previously documented; the walker accepts and logs the word
  rather than refusing on an unrecognized one).
- **One parser reaches both Logic and GarageBand.** The container is byte-identical;
  GarageBand's `.band` package differs only in the version word.
- Logic keeps **9–10 rotating full `ProjectData` backups** (`Backup/00`–`09`, depth
  varies, and some projects have no backups folder at all) plus Alternatives, entirely
  unprompted — this is the "you already have history" first-run value the passive slice
  depends on, and it costs the user nothing to obtain.
- **GarageBand does not share this.** *Measured* on three real `.band` packages:
  GarageBand keeps no `Project File Backups` folder at all. It has an `Autosave/`
  directory of timestamped `.songData` files instead, but only one existed across the
  whole library sampled. GarageBand's day-one pitch is therefore "compare + passive
  history from today forward," not "here are the backups you already made" — a real,
  *measured* asymmetry with Logic, not an assumption.
- Track names are readable at a corrected offset (record start + 0x34) and region names
  from `AuRg` payloads on real material, but the raw `qeSM` name set mixes user tracks
  with system entries and MIDI-region names — any name shown to a musician as a "track"
  needs the `karT`↔`qeSM` pairing and a system-name filter, not a raw dump. This
  re-confirms the existing census-noun ban (below): entity counts read off the container
  are not user-facing track counts.

Logic-parameter-level diff (Ableton's "Semantic" tier) is still not available — Logic
payload schemas are still unmapped, i.e. issue #3 is still open. That gap is why this ADR
scopes Logic to a *structural* honesty tier (tracks/regions/audio files added, removed,
renamed; tempo) rather than claiming parity with Ableton's parameter-level diff.

## Decision

**The first shipped product is a macOS, Tauri-based, menu-bar-first app that gives Logic
Pro users a read-only "what changed" comparison over history that already exists on
disk, with GarageBand and Ableton supported at the tiers their formats currently allow,
and zero-install HTML sharing for recipients on any platform.**

Concretely:

1. **Logic is the first *consumer* target**, on the Structure honesty tier: real
   track/region/audio-file names, added/removed/renamed, plus tempo — never a parameter
   claim, never "no musical change" (only the narrower, honest "no structural change
   detected — Wit can't yet see knob and fader moves in Logic"). GarageBand rides the
   same parser at the same tier, with onboarding copy that does not promise pre-existing
   backups it does not have.
2. **Ableton keeps its Semantic tier** (full change vocabulary, including device-setting
   fingerprints) as designed in the frozen prototypes — nice-to-have for this cohort, not
   the lead surface.
3. **Every DAW gets the Ears tier for free**: bounce-vs-bounce null-diff with no parser
   at all, because audio needs no format-specific reader.
4. **No project-file write-path in this slice.** Restore means reveal-in-Finder or
   copy-with-caveats, never an in-place edit — this is what makes shipping to Logic users
   safe before a single byte of Logic's format is ever *written*.
5. **No commit/branch/merge UI.** The zero-Alternatives-uses datum says that surface is
   declined; it is not resourced in this slice.

## Scope relative to ADR-0005

**This ADR does not overturn ADR-0005.** ADR-0005 answers "which DAW format do we parse
and eventually write to, for the version-control roadmap" — that stays Ableton-first,
unchanged, because Ableton is still the only format with a demonstrated parse → diff →
merge → repack chain. This ADR answers a narrower, different question: "which DAW's
users get the first *read-only* product." The two questions have different answers
because the read-only slice does not need merge, does not need a write path, and does
not need parameter-level parsing to be honest — it needs only structural facts, which
Logic already provides. ADR-0005's write-path roadmap (Ableton → Logic/GarageBand → FL
Studio) is explicitly unchanged and is exactly what ADR-0005's second overturn
condition anticipated: *"A Logic `ProjectData` payload map arriving from the community
that makes Logic's diff as good as Ableton's."* That map has not arrived; this ADR does
not claim it has. It ships a smaller, honestly-scoped product on the format evidence that
does exist today.

## Consequences

**Good**

- Ships to an identified, willing cohort instead of an anonymous one; unlocks the exit
  criterion (5 producers, 2 unprompted compares/week) with real usage data instead of
  more research.
- Read-only means ROADMAP blocker #1 does not gate this slice, and restore-as-reveal
  keeps the "never modify a user's DAW projects" rule (AGENTS.md) true by construction.
- One parser (Logic/GarageBand container) reaches two DAWs and, per the probe, a stock
  Mac's built-in DAW — the widest reach of any format currently readable.
- The honesty tiers (Structure / Semantic / Ears) make "never claim more than the parser
  sees" a per-DAW, type-level property instead of a per-feature judgment call.

**Bad, and accepted**

- Logic's Structure tier cannot see knob/fader moves; a save that is musically
  significant but structurally silent will show as "no structural change detected."
  This is stated honestly in the product, not hidden — and is the same gap issue #3 was
  already tracking.
- GarageBand's day-one value is thinner than Logic's (no rotating backups) — onboarding
  copy must not paper over this with Logic's pitch.
- Ableton, the format with the best diff, is deprioritized as a *consumer* surface for
  this slice even though its parser is the most complete. This is a genuine trade against
  ADR-0005's own reasoning, made because market/cohort evidence (who will actually use
  it) now outweighs format readiness for the read-only product specifically.

## What would overturn this

- The friend cohort turns out to skew Ableton, not Logic — the format-readiness argument
  from ADR-0005 would then apply directly to the consumer surface too.
- A Logic `ProjectData` payload map lands (issue #3) and Logic reaches the Semantic tier
  — the honesty-tier distinction between Logic and Ableton in this ADR would collapse,
  though the Logic-first *consumer* choice would likely still stand on cohort grounds.
- The hands-off-week experiment (ROADMAP's "The metric that matters") comes back negative — a clean
  negative on comprehension redirects investment to Logic payload schemas before more
  product, per the plan; it would not automatically revive branch-and-merge as the
  headline, since the zero-Alternatives-uses datum is unaffected by a comprehension
  result either way.

## Related

[ADR-0005](0005-first-daw-target.md), [FORMATS.md](../FORMATS.md),
[ROADMAP.md](../ROADMAP.md).
