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
readable "what changed" comparison. No project-file write-path, so [issue #1](https://github.com/sep-lab/Wit/issues/1)
(byte-faithful round-trip) does not gate it — that blocker applies to the write-path
roadmap below, not this slice. Full rationale, the adoption counter-evidence that drove
the pivot, and what would overturn it are in the ADR.

**Goal: install Wit, point it at a Logic library, and read a comparison a musician can
understand — without ever risking a project file.**

| Milestone | Tracking | What it delivers | Exit criterion |
|---|---|---|---|
| M0 ✅ | [PR #13](https://github.com/sep-lab/Wit/pull/13) | ADR-0006, Rust workspace scaffolding, CI (`rust` + `licenses` jobs) | Landed |
| M1 ✅ | [PR #24](https://github.com/sep-lab/Wit/pull/24) | Ableton `.als` parity port (`wit-als`) — Rust port of the working Python differ | Landed — golden byte-for-byte (`crates/wit-diff/tests/golden.rs`); spot-checked against a real 29-save `Backup/` chain via `WIT_FIXTURES` (28 pairs parse clean, no panics; 2 pairs cross-checked line-for-line against Python's actual output, exact match). The formal corpus-agreement gate against the specific 7 zero-change / 3 knob-only pairs named in `wit-planning/PLAN.md` has not been re-run — flagged for a follow-up pass |
| M2 ✅ | [PR #25](https://github.com/sep-lab/Wit/pull/25) | Logic/GarageBand `ProjectData` walker (`wit-logic`) | Landed — spot-checked against a real `.logicx` project + its 9 on-disk `Project File Backups` (10 files, clean EOF on all, tempo matches `MetaData.plist` exactly on all); 5 of 9 real consecutive pairs correctly report `NoStructuralChange` despite differing bytes. The 30-fixture `jonkubis/LogicProFormatWriter` corpus fetch (pinned SHA `1f77c5c37d49ccd9551cc8e9107750e8db2f1fed`) has not been run — network-gated, opt-in, flagged for a follow-up pass |
| M2.5 | [#15](https://github.com/sep-lab/Wit/issues/15) | **Reality gate** — measure the empty-verdict rate on a real Logic library | Published finding in EXPERIMENTS.md; if >50% of saves show no visible structural change, the next work is mapping Logic's volume-fader field, not the GUI |
| M3 | [#16](https://github.com/sep-lab/Wit/issues/16) | Index, content-addressed store, CLI (`wit-index`, `wit-cli`) — covers Logic, GarageBand, and Ableton discovery | `wit scan` lists every project with sane version counts; `wit dupes` reproduces the ~5.4 GB figure; rescan idempotent |
| M4 | [#17](https://github.com/sep-lab/Wit/issues/17) | Audio engine — decode, peaks, alignment, null-diff (`wit-audio`) | Injected sample shifts recovered exactly; two 5-min bounces diff in < 10 s |
| M5 | [#18](https://github.com/sep-lab/Wit/issues/18) | Tauri app alpha — Shelf, Story, Compare, watcher | Installs on a second Mac, reads a diff on the demo library, a real Logic folder, **and one Ableton `.als` lineage**, clicks Reveal |
| M6 | [#22](https://github.com/sep-lab/Wit/issues/22) | Share bundle + zero-install viewer (`wit-share`, `viewer/`) | AirDrop the `.html` to another Mac **and** open it on a physical iPhone via iMessage/Quick Look — sentences, waveform, and audio all render with JS off; zero network requests |
| M7-lite | [#19](https://github.com/sep-lab/Wit/issues/19) | **Unsigned** pilot package + PILOT.md | Clean Mac installs from the release link using only PILOT.md — recipients will see macOS's unsigned-app warning and use System Settings → Privacy & Security → "Open Anyway"; PILOT.md walks through it. Notarized distribution (Apple Developer ID) is a 0.1 upgrade, not a 0.0 blocker |

Full milestone detail lives outside this repo per ADR-0006's evidence trail; see
`AGENTS.md`'s settled-decisions table for what's locked. (The Ableton port and share
viewer were briefly deferred to 0.1 in an earlier pass of this table; both are back in
the 0.0 pilot as of the M0.5 truth-up.)

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
