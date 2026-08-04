# Roadmap

Wit is in the **design phase**. The research is done and the architecture is decided;
what follows is building it.

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
- [x] Model a real 26 GB library: 8.1 GB with full history

Output: [EXPERIMENTS.md](EXPERIMENTS.md), [FORMATS.md](FORMATS.md),
[decisions/](decisions/), `experiments/`.

---

## Phase 1 — Single-player Ableton

**Goal: a producer installs Wit, keeps working exactly as before, and gains a complete,
readable history of their project.** No collaborator required. This is the whole bet — if
it is not useful alone, nothing later matters.

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

---

## Phase 2 — Two people, one DAW

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

---

## Phase 3 — Logic and GarageBand

One parser covers both — same container, same magic bytes.

- [ ] `ProjectData` chunk-payload schemas beyond the container (container is already
      decoded; payloads are not)
- [ ] Handle non-deterministic save churn — the regenerated plugin UUID and the drifting
      float32 block
- [ ] Map `Alternatives/` onto Wit branches (Logic already branches; it just cannot
      compare or merge)
- [ ] GarageBand support falls out of the same parser

---

## Phase 4 — FL Studio

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

Phase 0's numbers come from a handful of projects on one machine. **Breadth is the
biggest gap**, and it needs no Rust:

- Run `experiments/` against your own sessions and report what you get
- Tell us how your studio actually collaborates
- Map more Logic chunk payloads, or model device parameters in the Ableton extractor

See [CONTRIBUTING.md](../CONTRIBUTING.md) and the task table in the
[README](../README.md#where-to-start-contributing).
