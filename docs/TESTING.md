# Testing strategy

**Status: none of this exists yet.** Wit has zero tests. CI checks that the prototypes
compile, that no audio is committed, and that doc links resolve — nothing about
correctness. This document is the plan, and it is written in the order the failure modes
deserve, not in the order a testing pyramid is usually drawn.

Labels follow the repo convention: **measured** (we ran it), **cited**, **inferred**.

---

## 1. What we are actually defending against

A version control system for music has an unusual risk profile. Ranked:

| # | Failure | Consequence | Detectable by CI? |
|---|---|---|---|
| 1 | **Silent data loss** — Wit writes a project the DAW rejects, or opens with content missing | Someone's work is gone | Only by proxy |
| 2 | **A wrong published number** | The repo's entire credibility | Direction and magnitude only |
| 3 | **Parser crash / hang / unbounded allocation on hostile input** | [SECURITY.md](../SECURITY.md) scope | Yes, by fuzzing |
| 4 | **Noise** — phantom commits, unreadable diffs | Product is useless but not dangerous | Yes |

Everything below derives from that ranking. If a testing activity does not defend one of
those four, it is optional.

### Why #1 dominates

The asymmetry is total. A VCS that fails loudly costs an afternoon; a VCS that quietly
drops an automation lane costs a take that no longer exists. Worse, adoption makes it
worse: the whole pitch is "stop keeping `Final_v3_REAL_final_2.zip`", so the moment Wit
succeeds, the user's redundant copies are gone. Wit inherits the obligation those copies
were discharging.

And it is *silent* by nature. Ableton's XML has no published schema
([ADR-0005](decisions/0005-first-daw-target.md)); parsing is empirical. A parser that
drops an element it does not recognise produces a file that opens fine, plays fine, and
is missing a send. Nobody notices for weeks. There is no crash to catch.

The architecture already commits to the mitigation — *"the verbatim original project
file, delta-chained"*, and *"unmodelled bytes round-trip untouched"*
([ARCHITECTURE.md](ARCHITECTURE.md) §1). The testing job is to make that claim
falsifiable rather than aspirational.

### The oracle problem, stated plainly

The real acceptance criterion is: **does Ableton Live 12.3.5 open this file, and is the
music unchanged?** CI cannot evaluate that. There is no headless Live, no licence server
we can legally drive, and half the criterion is perceptual.

So the strategy splits in two, and neither half substitutes for the other:

- **CI proves the strongest available proxy: byte identity.** `serialise(parse(f)) == f`,
  exactly, for every byte. We choose byte equality over "it still parses" or "it is
  schema-valid" because equality is decidable and validity is not — we do not have the
  schema, and Live is the only thing that defines it.
- **A human with the DAW proves acceptance**, on a named build, as a release checklist
  (§7). It is n=1 per release. It is also the only test of the actual requirement.

Byte identity is a proxy, and we should say so out loud: a file can be byte-identical and
still be wrong if Wit *chose* the wrong version to restore. Byte identity defends the
serialiser; the integration tests (§5) defend the choice.

---

## 2. Defending the numbers

Failure mode #2 is different in kind and needs its own regime.
[AGENTS.md](../AGENTS.md) already forbids inventing a benchmark figure and forbids
benchmarking compression on synthetic audio. Both rules constrain what CI is allowed to
assert, and the constraint is sharper than it first appears.

**What is actually being defended is not "11 KB per save".** It is the set of *claims
that decide the architecture*:

| Claim | Test on synthetic fixtures | Test on real material |
|---|---|---|
| Delta chains beat CDC on project history | **Direction:** `delta_total < cdc_total < naive_total` | The 29× figure |
| A save changes a fraction of a percent | **Band:** per-save cost < 1% of file | The 0.07–0.25% figure |
| Global re-render leaves nothing reusable | **Bound:** CDC reuse < 1% | The 0.00% figure |
| Ableton element IDs are stable across saves | not testable — it is a property of Live | opt-in fixtures only |
| FLAC keeps ~47% on a real library | **must not be asserted at all** | opt-in, and quoted with material |

Two rules make this honest:

**Assert direction and order of magnitude, never the figure.** A CI assertion of
`delta < cdc` on a generated 30-save chain (base set, then 0.1%-of-lines mutations) is a
real invariant: it fails if someone breaks the delta path or the normalisation boundary.
An assertion of `11 KB ± 1 KB` on a synthetic set is theatre — the number is a property of
one real project, not of the algorithm.

**Synthetic audio is legitimate for exactly one of these and illegitimate for the rest.**
The global-re-render result is a statement about arithmetic: change the gain and every
sample value becomes a different number, so no byte sequence survives. That mechanism has
nothing to do with musical content, so generating PCM, applying a gain to every sample,
and asserting reuse < 1% is a valid regression test of the CDC implementation. The FLAC
ratio is the opposite: it depends entirely on the material — measured 1.1% on a
mostly-silent stem and 65.2% on a dense master ([EXPERIMENTS.md](EXPERIMENTS.md) §8).
Asserting a FLAC ratio on generated audio would be the exact error AGENTS.md prohibits.
The distinction is not "synthetic is bad", it is **does the mechanism under test depend on
the material?**

**When a band fails, the failure is the finding.** Mirroring AGENTS.md: do not widen the
band to make CI green. Report the movement, decide whether the doc number is now wrong,
and say so in the PR.

**Anti-rot check (cheap, add early).** A `docs-numbers` job that verifies every command
in EXPERIMENTS.md's "Reproducing these" block still exists and still runs `--help`. It
catches the rot where a script is renamed and a documented reproduction silently becomes
impossible. It does **not** verify any value; nothing in CI can.

---

## 3. Unit tests

Small, fast, no fixtures, no I/O. These are where the boring bugs die.

**Chunking** (`chunk_bounds` in `experiments/cdc_dedup.py`, and its Rust equivalent):

- boundaries **partition** the input — no gaps, no overlaps, `sum(len) == len(data)`
- every chunk except the last is within `[min_sz, max_sz]`
- determinism: same bytes in, same boundaries out, across runs and platforms
- degenerate inputs: empty, 1 byte, exactly `min_sz`, exactly `max_sz`, all-zeros
  (all-zeros is not academic — a Cubase sample file measured **89.3% zero bytes**)
- **the gear table is a storage-format constant.** Lock its hash in a test. Changing
  `GEAR` silently re-chunks everything and invalidates every stored object; that must be
  a loud, deliberate format-version change, not a diff nobody reads.

**Varint and event sizing** (`experiments/flp_parse.py`, and the Rust parser):

- single-byte, multi-byte, continuation-bit handling, and the ID→size boundaries at
  63/64, 127/128, 191/192 — off-by-one there mis-frames the entire remaining stream
- truncated varint → a typed error, not `IndexError` (**measured**: it is `IndexError`
  today, §6)
- an oversized declared length → refuse, do not allocate

**Diff primitives:**

- rename fan-out coalescing returns the right reference count (418, in the real case)
- track add / remove / rename classification
- output ordering is deterministic — the diff text is UI, and unstable ordering makes
  golden tests useless and users distrust the tool
- **float rounding is a correctness property, not formatting.** `_num(..., places=3)`
  decides whether `0.7940001` and `0.794` are "the same volume". Too tight and Wit reports
  phantom mix changes on every save; too loose and it misses a real automation-scale move.
  Pin the policy with tests at the boundary.

**Normalisation:** hashing must happen on decompressed content, never on the container.
This is called the highest-leverage detail in the design
([ADR-0002](decisions/0002-storage-model.md)); assert it directly — same logical content
in two different gzip encodings must produce the same object id.

---

## 4. Property-based tests

**Recommendation: Hypothesis** (Python) and `proptest` (Rust). Hypothesis in particular,
because the interesting generator here is not "random bytes" but "a valid-ish session
model", and `st.builds` over a model type plus a serialiser is exactly the shape it is
good at.

**Dependency note, since it looks like a conflict:** AGENTS.md forbids dependencies in
`experiments/`. That constraint is about the prototypes staying runnable by a musician
with a stock Mac — it is not a constraint on a `tests/` directory. Tests live outside
`experiments/`, declare `pytest` and `hypothesis` in their own requirements file, and
import the prototypes as modules. `experiments/` gains no dependency and stays runnable
with bare `python3`.

### The properties, in priority order

**P1 — Round-trip identity.** `serialise(parse(f)) == f`, byte for byte, for every
generated project. This is the single most important test in the repository. It is the
machine-checkable form of "Wit must never be the reason a session fails to open".

**P2 — Bounded mutation.** Parse, change exactly one modelled field, serialise. The byte
diff against the input must be confined to that field's bytes. This catches the whole
class that P1 misses: attribute reordering, namespace rewriting, changed float
formatting, changed line endings, re-indentation. Each is individually harmless-looking
and collectively how you produce a file Live refuses.

**P3 — Unmodelled fields survive.** Inject a random unknown element and a random unknown
attribute into the generated project; require both to appear unchanged in the output.
This is the direct test of the hard rule in ADR-0001 and
[ADR-0003](decisions/0003-plugin-state-policy.md), and it is the property that protects
against Live schema drift, which is guaranteed to happen.

**P4 — CDC insert-invariance.** Insert *k* bytes at offset *o*: every chunk lying wholly
before *o* is unchanged, and the number of changed chunks is bounded. This is the property
that made CDC worth having at all (measured 99.6% reuse on a time-shifted region), and it
is easy to break with an innocent-looking rolling-hash change.

**P5 — Delta and chain round-trip.** `apply(diff(a, b), a) == b` for arbitrary pairs,
including `a == b`, empty inputs, and single-byte inputs. Then the chain form: build an
*N*-version chain and restore **every** index, asserting byte identity at each. That test
is what exercises checkpointing, and ADR-0002 explicitly warns that "corruption
propagates" down a chain.

**P6 — Disjoint edits always merge.** Base *B*; an edit confined to track *X*; an edit
confined to track *Y*, *X ≠ Y*. Merge must succeed and contain both. EXPERIMENTS.md §5
demonstrates this once, by hand, with `git merge-file`. As a property over generated
sessions it becomes a real guarantee — and it is the guarantee the whole collaboration
story rests on.

**P7 — Commutativity, but only where it should hold.** `merge(A, B) == merge(B, A)` for
tier-1 (auto-merge) cases only. State the exclusions in the test file, because asserting
them would encode bugs as requirements:

- tier-2 same-parameter conflicts resolve by human choice, and are order-dependent by
  design
- ordered containers — device chain order, FL's positional event stream — are not
  commutative and must not be forced to be

**P8 — Merge identity.** `merge(B, B, B) == B`. Merging a change with itself is a no-op.
Trivial, and it catches a surprising number of three-way implementation errors.

**P9 — Diff soundness, in one direction only.** If two parsed models are equal, the diff
must be empty. Do **not** assert the converse. Measured: three consecutive Logic saves had
identical record censuses and no semantic change, yet differed by 97 and 140 bytes,
because a plugin UUID is regenerated every save and ~20 float32s are rewritten at a fixed
stride. "Bytes differ ⟹ diff non-empty" is *false by design*, and it is the same fact as
ADR-0002's rule that dirty detection must be semantic. Encode the asymmetry in the test,
with the reason in a comment.

**P10 — Store integrity.** Content-addressed lookup returns an object whose recomputed
hash matches its id, always, with no fast path. Verification bypass is named in
SECURITY.md.

**P11 — FLAC round-trip is bit-exact, per file.** ADR-0002: *"FLAC round-trip must be
verified bit-exact per file, or Wit silently corrupts audio. This needs a test that
re-decodes and compares hashes, always, with no fast path."* Generate across 16/24-bit and
32-bit float, mono/stereo/multichannel, unusual sample rates, all-silence, full-scale, and
DC offset. This is the one place generated audio is unambiguously correct to use: the
property is exactness, not ratio.

### Using Hypothesis well here

- Generate **models**, then serialise — not raw bytes. Raw-byte generation is fuzzing's
  job (§6); property testing should spend its budget on the valid-input space.
- **Every falsifying example becomes a permanent `@example`** in the test file. CI is
  ephemeral, so the `.hypothesis` example database does not survive; a bug found and not
  pinned will be found again in a year. Committing the shrunk example as code is the
  durable form.
- `deadline=None` for anything shelling out to `zstd` or `flac`; timing on shared runners
  is noise.
- `max_examples` low on PRs (fast feedback), high on the nightly job (real search).

---

## 5. Golden tests and integration

### Golden / characterisation

**The diff output is the product.** `wit diff` is the first thing anyone sees, and its
text is a user interface. Golden files hold the rendered diff text for a set of synthetic
sessions — plain `.txt`, tiny, no audio, no project files, so they are CI-legal by
construction.

Rules:

- goldens are regenerable by one documented command
- a changed golden is a **user-visible change** and must be reviewed as one; a golden diff
  with no note in the PR is a review failure
- the two hardest documented behaviours get permanent goldens, because if they regress the
  product regresses: the sample-rename fan-out collapsing 425 clip changes into one line,
  and `no musical change detected (view / bookkeeping only)` for a view-state-only edit

**Characterisation tests for parsers** pin the *census* — record counts by chunk tag,
event counts by ID — rather than the full structure. A parser change that quietly stops
recognising a chunk class then fails loudly instead of shrinking a number nobody was
watching.

### Integration

Full cycle on generated projects in a temp directory:

```
init → commit ×N → log → diff → restore k, for every k
```

with byte-identity asserted at every *k*. That is the end-to-end statement of "no data
loss", and it is the test that exercises delta chains, checkpointing, and the commit DAG
together.

Also at this layer, because they need a filesystem:

| Case | Required behaviour |
|---|---|
| Restore into a dirty working tree | Refuse. Never overwrite unversioned changes. |
| Sample path containing `../..`, an absolute path, or a symlink | Nothing is written outside the working directory. SECURITY.md names path traversal explicitly; a project file is attacker-controlled input. |
| Referenced sample missing | Clear message; the reference is preserved, never silently dropped |
| Watcher sees *N* saves with no semantic change | **Zero** commits. The anti-spam property from ADR-0002. |
| Process killed mid-commit (injected fault) | Store is consistent on restart — temp file plus atomic rename, no torn objects |

---

## 6. Fuzzing

This is not speculative hardening. The parsers take hostile binary input by definition,
and today they fall over on trivial malformation.

**Measured** — Python 3.13.0, macOS, four hand-written malformed inputs, run against the
current prototypes:

| Input | Result |
|---|---|
| `.flp` with a truncated header | uncaught `struct.error` |
| `.flp` with a truncated varint length | uncaught `IndexError` |
| `.als` whose XML has no `<LiveSet>` | uncaught `AttributeError: 'NoneType' object has no attribute 'find'` |
| `.als` that is not gzip | uncaught `gzip.BadGzipFile` |

For the prototypes this is *acceptable and already documented* — SECURITY.md puts
`experiments/` explicitly out of scope, and each script warns not to run it on untrusted
input. For the Rust core it is a release blocker. The finding worth carrying forward is
that these four inputs took ten minutes to write and all four landed. A fuzzer will do
better.

**One measurement worth recording**, because it looks like a pass and is not: a
billion-laughs `.als` (189 compressed bytes, ~10 MB logical expansion) was **refused** —
`limit on input amplification factor (from DTD and entities) breached`. That protection
comes from the bundled libexpat, not from our code. CI pins Python 3.9, whose bundled
expat may differ. **Therefore assert it, do not assume it**: a test that feeds an
expansion bomb and requires rejection, on every supported runtime. XML entity expansion is
named in SECURITY.md; inheriting the defence silently from a transitive dependency is not
the same as having it.

### Targets and tooling

- **Rust core: `cargo-fuzz` / libFuzzer**, one target per parser entry point — gunzip
  container, `.als` XML→model, `.flp` event stream, `ProjectData` chunk reader, delta
  apply, object-id decode.
- **Structure-aware generation** via `arbitrary` for the event stream and chunk reader, so
  the fuzzer spends its budget past the magic-bytes check instead of rediscovering
  `FLhd`.
- **Atheris** for the Python prototypes is optional — out of scope per SECURITY.md — but
  worth wiring if someone wants it, because the corpus is shared.

### Oracles beyond "did not crash"

- **Bounded allocation.** libFuzzer's `-malloc_limit_mb`. "Unbounded allocation" is listed
  in SECURITY.md as a vulnerability in its own right; a parser that allocates a declared
  4 GB length has failed even if it never crashes.
- **Bounded time.** Timeout per input; a hang on a malformed file is a denial of service
  on a background daemon.
- **Differential.** ADR-0004 leaves us with two implementations of the same parsers — the
  Python prototype and the Rust core. Feeding both the same input and requiring the same
  census is a free, strong oracle that finds silent divergence, which is exactly the
  failure mode of §1. Keep the prototypes alive for this reason if for no other.

### Corpus strategy — this is the fixture problem again

A fuzzing corpus is normally seeded from real files. **Ours cannot be.** A minimised crash
input derived from a real project can still carry sample names, region names, and absolute
paths — measured, one real `.als` embeds 777 references under a stranger's home directory.
Committing that corpus would leak three people's data and probably some third party's
copyright, and git history is forever.

Rules:

1. **Seeds are generated, not collected.** The synthetic fixture generator (§7) produces
   the seed corpus; what is committed is the generator and its seeds, never the bytes.
2. **A crash reproducer is committed only after it is reduced to a synthetic minimal case**
   that shares no string with the original file. In practice, most reduce to a handful of
   bytes and this is easy.
3. **If a reproducer cannot be sanitised, it does not enter the repo.** It goes to the
   private advisory under SECURITY.md, and the public regression test uses a synthetic
   equivalent that triggers the same code path.
4. **Corpus growth lives in CI cache**, not in git.
5. A crash found by fuzzing is reported through private vulnerability reporting, not as a
   public issue — SECURITY.md's process, not an exception to it.

**Where it runs:** nightly and weekly, never on PRs — fuzzing on a PR budget finds
nothing and flakes. OSS-Fuzz once there is a released artifact to submit.

---

## 7. Fixtures — the defining constraint

Everything above assumes fixtures exist. Getting this right is the central design problem
of the suite, because three independent constraints all point the same way:

- **Policy.** Never commit audio (AGENTS.md), enforced by the existing CI grep.
- **Copyright.** A real project embeds third-party sample names and structure, and
  frequently other people's absolute paths.
- **Size.** The repo-size job caps `.git` at 50 MB; one real Ableton set is 473 KB
  compressed and a real Logic project is 459 MB.

### Tier 0 — synthetic, generated, committed as code (CI)

**The generator is the fixture.** A committed script emits `.als` / `ProjectData` / `.flp`
byte streams from parameters: track count, clip count, device chains, sample references,
and a mutation function that applies a realistic single edit.

**Measured** (this machine, Python 3.13): a synthetic 20-track × 30-clip Live set is
**189 KB of XML, 3.5 KB gzipped**, and the existing `als_semantic_diff.py` parses it and
reports the expected mix change unmodified. Generating a 30-save chain in CI costs
milliseconds and zero repository bytes.

Rules:

- **Never commit generated bytes. Commit the generator.** A fixture is identified by
  *(generator version, seed, parameters)* — which is also what makes a failure
  reproducible.
- Generators are seeded and deterministic across platforms.
- This keeps the CI audio/project-file grep true by construction rather than by vigilance.
- The manifest `experiments/fixtures.toml` is already referenced from `.gitignore` and does
  not exist yet; that is the natural home for fixture declarations.

**Limits, stated honestly.** A synthetic Live set has none of the real schema's quirks,
and CI passing on Tier 0 proves *invariants only* — never magnitudes, never that Live
accepts anything. Tier 0 is a regression net, not evidence.

### Tier 1 — real fixtures, opt-in via environment variable (local)

`WIT_FIXTURES=/path/to/Project/Backup`. Tests needing real material skip when it is unset.
This tier is where the published numbers come from, where ID stability is checked, and
where schema drift is discovered.

#### LOUD SKIPS — a rule, not a nicety

**The failure this exists to prevent is documented in our own FORMATS.md.**
`logic2ableton` is widely cited as proof that Logic→Ableton conversion is solved. It is
not: its MIDI extraction recovers **zero notes** from real modern Logic projects — its
hardcoded 15-byte signature does not occur once across 30 genuine fixtures — and **its
tests against real projects auto-skip.** The headline capability was broken, the test
suite was green, and the discrepancy survived because nobody was told the tests had not
run.

Wit is *more* exposed to this than most projects, because most of our test material can
never live in the repo. So:

1. Skipping prints a **warning line naming the environment variable** and the count, in
   the run summary, not buried in per-test output.
2. The suite emits a distinct summary line:
   `real-fixture tests: 0 run, 37 skipped (WIT_FIXTURES unset)`.
3. **CI job names encode it** — the fixtureless job is named
   `tests (synthetic fixtures only)`, so a green tick can never be misread as "verified
   against real projects".
4. **A never-run test is a dead test.** Any Tier-1 test not executed by a human in the last
   two releases is listed in the release checklist as unverified.
5. The release checklist requires a full Tier-1 run on a machine that has the material,
   with output pasted into the release PR.
6. No `skip` or `xfail` without a reason string. Ever.

### Tier 2 — community-contributed sessions

CONTRIBUTING.md already asks for these and already states the licence rule. The additions
needed for testing:

- **Never into this repository.** A separate, explicitly-licensed corpus repo, or an
  archive fetched by URL and content hash at test time. The repo-size job would reject it
  anyway, and git history cannot be un-published.
- **Contributor must own it outright**, with no third-party commercial samples. A project
  that *references* samples by name without shipping audio is preferred and is usually more
  useful for diff testing.
- **Absolute paths are personal data.** Real project files carry a real person's name in
  hundreds of paths. Either the contributor runs a path-scrubbing pass, or the fixture is
  local-use-only and never redistributed. Make the choice explicit at contribution time,
  not later.
- **Prefer a census over a file.** The most valuable contribution is not a project — it is
  a contributor running the suite on their own sessions and pasting the report into a
  Measurement issue. It gets us breadth, which is the acknowledged weakness of every number
  in this repo, at zero licensing risk. That template already exists.

---

## 8. Human-in-the-loop: the DAW acceptance checklist

This is the only test that tests the actual requirement, and it cannot be automated. No
supported headless mode, licence terms, GUI-only interaction, and a partly perceptual
criterion. Attempting to automate it would produce something brittle that still would not
answer the question — so instead it is a **release checklist**, versioned in this repo,
executed by a named human, recorded with build numbers.

It maps directly to gates that already exist: ROADMAP Phase 1 exit criterion 1, the
release gate in EXPERIMENTS.md §5, and open question 1 in ARCHITECTURE.md §7.

### Recorded environment (per row)

DAW and **exact build** (e.g. Live 12.3.5, Logic 11.2.2, GarageBand 10.4.x, FL 24.x), OS
and version, and **CPU architecture** — architecture is not bookkeeping here: denormal
handling differs between x86 `MXCSR` and aarch64 `FPCR`, which is why render
reproducibility is an open question in EXPERIMENTS.md.

### Steps, per supported DAW

1. **Restore fidelity.** Commit an untouched real project; restore into an empty
   directory; open in the DAW. No missing-media dialog, no version warning, track / clip /
   plugin counts match, and a 30-second section plays as expected.
2. **Byte identity.** Compare the restored file to the original. It must be identical.
   *A file that opens but is not byte-identical is still a failure at this stage* — byte
   identity is the invariant we chose, and a serialiser that "usually works" is the
   silent-loss failure waiting to happen.
3. **Merge acceptance.** Two disjoint edits merged by Wit → opens in the DAW → both edits
   present, project plays. This is the item EXPERIMENTS.md §5 explicitly leaves unverified.
4. **Save-back round trip.** Open the restored project, save from the DAW, commit again.
   Wit must report no phantom changes beyond documented churn.
5. **Missing-plugin path.** Open on a machine lacking one referenced plugin; confirm Wit's
   pre-open warning matched reality (ADR-0003).

### Sign-off

A table in the release PR: DAW, build, OS, arch, tester, pass/fail, notes. **A release
with an unfilled row does not ship.**

**What this does not prove.** One project, one machine, one build — a spot check with
n=1. Breadth comes from the property tests; ground truth comes from here. Neither
substitutes for the other, and claiming otherwise would be the kind of overstatement the
rest of this repo avoids.

---

## 9. What we deliberately do not test

| Not tested | Why |
|---|---|
| **DAW automation / GUI driving** | No supported headless mode, licence terms, and extreme brittleness. Worse than useless: it would produce a green signal that still does not answer "does it open and sound right". §8 is the honest replacement. |
| **Plugin behaviour** | ADR-0003 makes plugin state opaque by decision. Testing that a compressor compresses is testing someone else's product. We test only that Wit never mutates the blob. |
| **Audio quality / perceptual similarity** | No PSNR thresholds, no "sounds the same" assertions. We assert bit-exact FLAC round-trip; anything softer is not a property we can defend. |
| **Render determinism** | Measured as an open question and answered *no* — denormals and plugin drift. Never hash a render as an oracle; any test that did would be flaky by physics, not by flakiness. |
| **Cross-DAW conversion** | Not a feature, permanently (ROADMAP "Not planned"). |
| **Network / server behaviour** | Local-first, no account, no server. There is nothing to test. |
| **Performance, as a merge gate** | Wall-clock on shared runners is noise. Benchmarks are *reported* and tracked over time; the only CI guard is a coarse pathological limit (say, 100× baseline), which is a hang detector, not a benchmark. |
| **Hardening of `experiments/`** | SECURITY.md puts the prototypes out of scope by decision. They get correctness tests (§2 defends their outputs); they do not get fuzzing budget. |

---

## 10. Coverage policy

Aggregate coverage percentage is a weak signal and trivially gamed. "We don't measure it"
is worse. The honest posture is **targeted floors where the failure mode is data loss, and
informational reporting everywhere else.**

- **Protected modules — parsers, serialisers, delta and object store: 90% line *and*
  branch.** Branch coverage matters more than line coverage here, and the reason is
  visible in §6: parser bugs live in the error paths that well-formed input never takes.
  Every one of the four measured crashes is an uncovered branch.
- **Everything else: 60% aggregate, informational**, not a merge gate.
- **The floor ratchets.** It may rise in a PR; it may never fall. Lowering it requires an
  explicit line in the PR description and a reviewer's agreement. This ends the argument
  about 0.3% without anyone having to have it.
- **Do not gate on the coverage delta of a diff.** It is noisy and it punishes refactors.
  Gate on the absolute floor for protected modules only.
- **The CI number understates real coverage**, because fuzzing and Tier-1 fixture runs do
  not happen in CI. Say so, and do not chase the number.

**What would overturn this:** if the 90% floor starts producing assertion-free tests
written to touch lines, drop the number and gate parsers on **mutation score**
(`cargo-mutants`) instead — nightly, not per-PR. Mutation testing is the honest successor
for exactly this code; it is simply too slow to be the first thing we build.

---

## 11. Definition of done for a PR

Two lists, because conflating them is how checklists rot. The first is machine-enforced;
the second requires a human.

### Enforced by CI

| Gate | Status |
|---|---|
| Prototypes compile and `--help` runs | exists |
| `shellcheck` clean | exists |
| No audio or DAW project file committed | exists |
| `.git` under 50 MB | exists |
| All doc links resolve | exists |
| Unit + property + golden + integration suite passes on Python 3.9 | **to add** |
| Skipped-test count reported in the job summary; fixtureless job is named as such | **to add** |
| Nothing binary under `tests/fixtures/` — generators only | **to add** |
| Coverage floors on protected modules | **to add** |
| `docs-numbers`: documented reproduction commands still exist and run | **to add** |

### Enforced by review

- Docs updated in the same PR if behaviour changed *(already in the PR template)*
- Claims labelled **measured** / **cited** / **inferred** *(already in the PR template)*
- A changed number carries the command that produced it *(already in the PR template)*
- **A changed golden diff is called out and justified** — it is a user-visible change
- **A new parser field has a P3 unmodelled-preservation test**, not just a happy-path one
- **A bug fix ships with the failing input pinned as an `@example`** or a regression test
- **A skip has a reason string**
- Format work satisfies the two boxes already in the PR template: unmodelled bytes
  round-trip verbatim, malformed input handled without crash or unbounded allocation

Those last two are the most important boxes in
[the PR template](../.github/PULL_REQUEST_TEMPLATE.md), and today they are self-reported.
**The whole point of this strategy is to make them machine-checked** — P1/P2/P3 for the
first, fuzzing for the second.

---

## 12. Order of implementation

There are zero tests, so sequencing matters more than completeness. Build in this order,
because it is descending order of what it defends:

1. **The synthetic fixture generator.** Nothing else can be built without it, and it is
   what makes the whole suite legal to commit.
2. **P1 round-trip identity**, and P2/P3 immediately after. Failure mode #1, directly.
3. **P5 delta and chain round-trip**, including restore-at-every-index. Failure mode #1,
   via the store.
4. **Integration: commit → diff → restore**, byte-identical at every version.
5. **Golden diff output**, including the fan-out and no-musical-change cases.
6. **Fuzz targets** for each parser entry point, plus the entity-expansion assertion.
7. **The DAW acceptance checklist**, before any release, however small.
8. **Coverage floors** last — a floor is meaningless until there is a suite to floor.

Deliberately *not* first: hardening the Python prototypes. They are out of scope by
decision, and effort there does not defend the Rust core.

---

## What would overturn this strategy

- **A headless, licence-compatible way to open a project in a real DAW.** That would move
  the acceptance test from §8 into CI and change everything downstream of it. Worth
  watching; we do not expect it.
- **A DAW publishing a schema.** Verbatim preservation stays either way, but validity
  becomes decidable and the round-trip proxy gets a stronger sibling.
- **Evidence that byte-identical round-trip is too strict** — a DAW that legitimately
  rewrites its own file on load such that identity is unattainable. Then the invariant
  weakens to "identical after DAW normalisation", which is a much worse place to be, and
  we should know it early.

## Related

[AGENTS.md](../AGENTS.md) · [CONTRIBUTING.md](../CONTRIBUTING.md) ·
[SECURITY.md](../SECURITY.md) · [EXPERIMENTS.md](EXPERIMENTS.md) ·
[ARCHITECTURE.md](ARCHITECTURE.md) · [ADR-0002](decisions/0002-storage-model.md) ·
[ADR-0003](decisions/0003-plugin-state-policy.md) ·
[ci.yml](../.github/workflows/ci.yml)
