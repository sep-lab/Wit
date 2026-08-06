# Roadmap

Wit's research is done and its architecture is decided. What follows is building the
first product.

The ordering principle: **be excellent for one person using one DAW before being mediocre
for many.** A collaboration tool that is useless until your collaborator also installs it
never gets its first user.

---

## Phase 0 — Research ✅ complete

Establish whether this is possible at all, on real projects.

- [x] Reverse-engineer Ableton `.als`, Logic `ProjectData`, GarageBand `.band`, FL `.flp`
- [x] Measure save-to-save change (0.07–0.25% of an Ableton project)
- [x] Confirm element IDs are stable across saves — the prerequisite for diff and merge
- [x] Prototype a semantic differ and run it on a real 10-save chain
- [x] Demonstrate a clean 3-way merge of two disjoint edits
- [x] Settle the storage question — delta chains beat CDC by 29× on project files
- [x] Establish that byte-level versioning of rendered audio cannot work (0% reuse on
      global re-render) — the finding that determines the architecture
- [x] Model a real library: 21.9 GB audio → 12.0 GB with full history (mostly dedupe + FLAC)

Output: [EXPERIMENTS.md](EXPERIMENTS.md), [FORMATS.md](FORMATS.md),
[decisions/](decisions/), `experiments/`.

---

## Now: the 0.0 pilot

**[ADR-0006](decisions/0006-consumer-surface-logic-first.md) changed the plan.** The first
*shipped product* is not the write-path roadmap below — it's a Logic/GarageBand-first,
**read-only** macOS app: passive auto-history over backups Logic already keeps, plus a
readable "what changed" comparison. No project-file write-path, so blocker #1 (below)
does not gate it. Full rationale, the adoption counter-evidence that drove the pivot, and
what would overturn it are in the ADR.

**Goal: install Wit, point it at a Logic library, and read a comparison a musician can
understand — without ever risking a project file.**

| Milestone | Tracking | What it delivers | Exit criterion |
|---|---|---|---|
| M0 ✅ | [PR #13](https://github.com/sep-lab/Wit/pull/13) | ADR-0006, Rust workspace scaffolding, CI (`rust` + `licenses` jobs) | Landed |
| M2 | [#3](https://github.com/sep-lab/Wit/issues/3), [#5](https://github.com/sep-lab/Wit/issues/5) | Logic/GarageBand `ProjectData` walker (`wit-logic`) | Census matches published `AuRg`/`Trak`/`AuFl` trajectories; identical-save pair → `NoStructuralChange`; 30 real fixtures walk clean |
| M2.5 | [#15](https://github.com/sep-lab/Wit/issues/15) | **Reality gate** — measure the empty-verdict rate on a real Logic library | Published finding in EXPERIMENTS.md; if >50% of saves show no visible structural change, the next work is mapping Logic's volume-fader field, not the GUI |
| M3 | [#16](https://github.com/sep-lab/Wit/issues/16) | Index, content-addressed store, CLI (`wit-index`, `wit-cli`) | `wit scan` lists every project with sane version counts; `wit dupes` reproduces the ~5.4 GB figure; rescan idempotent |
| M4 | [#17](https://github.com/sep-lab/Wit/issues/17) | Audio engine — decode, peaks, alignment, null-diff (`wit-audio`) | Injected sample shifts recovered exactly; two 5-min bounces diff in < 10 s |
| M5 | [#18](https://github.com/sep-lab/Wit/issues/18) | Tauri app alpha — Shelf, Story, Compare, watcher | Installs on a second Mac, reads a diff on the demo library and a real Logic folder, clicks Reveal |
| M7-lite | [#19](https://github.com/sep-lab/Wit/issues/19) | Unsigned pilot package + PILOT.md | Clean Mac installs from the release link using only PILOT.md |

Deferred to 0.1: the Ableton parity port (M1) and the zero-install share-HTML viewer (M6).
Full milestone detail lives outside this repo per ADR-0006's evidence trail; see
`AGENTS.md`'s settled-decisions table for what's locked.

**The metric that matters:** five friends who are not contributors open a comparison twice
in a hands-off week, unprompted. Every other criterion here would pass even if nobody
wanted the product. See "The evidence that most challenges this project" in
[DECISION-DOC.md](DECISION-DOC.md).

---

## After 0.0 — the write-path roadmap

The phases below predate ADR-0006 and describe a different, larger bet: full version
control with a write-path, merge, and eventually multi-DAW support. They are **not
superseded** — ADR-0006 explicitly does not overturn [ADR-0005](decisions/0005-first-daw-target.md)
— but they are **not being worked on until after the 0.0 pilot reports back**. A clean
negative on the pilot's comprehension bet redirects investment to Logic payload schemas
(issue #3) before this resumes; a positive result funds it directly.

### Phase 1 — Single-player Ableton

**Goal: a producer installs Wit, keeps working exactly as before, and gains a complete,
readable history of their project, with a write-path.** No collaborator required.

- [ ] `wit` CLI skeleton in Rust ([ADR-0004](decisions/0004-implementation-stack.md))
- [ ] Content-addressed object store: BLAKE3, algorithm-tagged, delta chains with
      checkpointing
- [ ] Ableton parser → session model, with **verbatim preservation** of everything
      unmodelled
- [ ] Semantic diff, including fan-out coalescing and honest "no musical change"
- [ ] `wit watch` — daemon that auto-commits on save, using a stat cache and
      **semantic** dirty detection (a changed hash is not a changed song)
- [ ] `wit log` / `wit diff` / `wit restore`
- [ ] Audio: whole-file content addressing, FLAC with **verified bit-exact round-trip**
- [ ] Renders/freezes classified as cache, excluded from history

**Exit criteria**

1. A restored project **opens in Ableton Live and is byte-faithful**. Non-negotiable.
2. Full history of a real project costs < 50 KB per save.
3. The differ agrees with subtree hashing on which saves are musically empty — closing
   the known device-parameter gap.
4. **Five producers who are not contributors run `wit diff` twice in a week, unprompted.**

### Phase 2 — Two people, one DAW

Collaboration where both use Ableton — the smaller problem, solved properly.

- [ ] Track-granular 3-way merge on the object model
- [ ] Conflict tiers: auto-merge / ask the human / refuse (tempo, time signature, rate)
- [ ] **Exclusive locking** for unmergeable artifacts (Perforce's answer, not git's)
- [ ] Sample resolution by content hash — using Ableton's own `OriginalFileSize` +
      `OriginalCrc` rather than inventing a scheme
- [ ] "This version needs 2 plugins you don't have" before opening, not after
- [ ] Sync over plain object storage; lazy fetch of only the referenced stems

**Exit criteria:** two people edit different tracks of one session, both push, and the
merged session opens correctly in Live with both sets of changes.

### Phase 3 — Logic and GarageBand (write-path)

One parser covers both — same container, same magic bytes. The 0.0 pilot's `wit-logic`
crate is read-only groundwork for this phase, not a substitute for it.

- [ ] `ProjectData` chunk-payload schemas beyond the container (container is already
      decoded; payloads are not)
- [ ] Handle non-deterministic save churn — the regenerated plugin UUID and the drifting
      float32 block
- [ ] Map `Alternatives/` onto Wit branches (Logic already branches; it just cannot
      compare or merge)
- [ ] GarageBand support falls out of the same parser

### Phase 4 — FL Studio

- [ ] Event-stream parser for ≤ v24, where locality is proven (17 of 53,448 bytes
      changed between real autosaves)
- [ ] Inflate zlib-wrapped plugin state before hashing
- [ ] **Open problem:** the v25 offset-dependent scalar keystream. Until solved, restrict
      diffing to variable-length events.

---

## Not planned

Stated so nobody builds them by accident:

- **Cross-DAW project conversion.** Plugin state is opaque and unportable; anyone
  promising otherwise is guessing. See [ADR-0003](decisions/0003-plugin-state-policy.md).
- **Real-time collaborative editing.** A different product with different physics.
- **Hosting other people's copyrighted samples.**
- **Pro Tools `.ptx` deobfuscation.** Legal exposure outweighs the benefit; storage-level
  support needs no decryption anyway.
- **Being a DAW.**

---

## How you can help now

The 0.0 pilot's issues are tracked with `m2`–`m7` labels and milestones. Beyond writing
Rust, the biggest gap is still breadth — Phase 0's numbers come from a handful of
projects on one machine, and none of the read-only-slice measurements need Rust either:

- Run `experiments/` against your own sessions and report what you get
- Tell us how your studio actually collaborates
- Map more Logic chunk payloads, or model device parameters in the Ableton extractor

See [CONTRIBUTING.md](../CONTRIBUTING.md) and the task table in the
[README](../README.md#where-to-start-contributing).
