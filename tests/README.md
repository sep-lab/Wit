# Tests

Tests for the prototypes in `experiments/`. They exist because `docs/EXPERIMENTS.md`
turns those scripts' output into design decisions — ADR-0002 was overturned by one of
them — and a number produced by a subtly wrong parser is worse than no number at all.

## Run them

```bash
python3 -m pip install -r tests/requirements-dev.txt
pytest tests/
```

Coverage:

```bash
pytest tests/ --cov=experiments --cov-report=term-missing
```

Faster loop (skips the shell benchmark and the DoS timing test):

```bash
pytest tests/ -m "not slow"
```

The prototypes stay standard-library-only — that constraint is from `AGENTS.md` and is
enforced by `test_repo_hygiene.py`. It applies to `experiments/`, not to this directory;
the tests may use pytest.

## Run them against your own sessions

The suite ships no project files, so by default nothing here has ever seen a file a DAW
wrote. To fix that, point it at real material:

```bash
WIT_FIXTURES=~/Music/YourProject/Backup pytest tests/
```

`WIT_FIXTURES` is a directory that is searched (up to 8 levels deep) for `*.als`,
`*.flp` and Logic `ProjectData` files. Nothing is written to it — the tests fingerprint
every file they touch and assert it is unchanged, and anything needing a writable copy
copies into `tmp_path` first.

Without it, the run ends with:

```
====================== REAL-FIXTURE COVERAGE DID NOT RUN =======================
  12 test(s) that exercise real DAW material were skipped.
  ...
```

That banner is deliberate. A neighbouring project (`logic2ableton`) had real-fixture
tests that skipped silently in every environment; the suite read as green for months
while a format bug lived through all of it. Add `WIT_REQUIRE_FIXTURES=1` to turn the
skip into a failure — that is what a nightly job or a release gate should use.

## What is in here

| File | What it covers |
|---|---|
| `factories/als.py` | Builds synthetic gzipped `.als` XML from a declarative description |
| `factories/flp.py` | Builds byte-exact `.flp` streams, including deliberately corrupt ones |
| `factories/binary.py` | Seeded byte streams for the chunking / dedup properties |
| `test_factories.py` | Round-trips the factories through the real parsers |
| `test_cdc_chunking.py` | Boundary stability, size bounds, cross-process determinism |
| `test_cdc_dedup.py` | Reuse and store-cost arithmetic against hand-computed answers |
| `test_als_model.py` | Which fields the semantic model extracts, and which it must ignore |
| `test_als_diff.py` | One isolated edit at a time, whole-output assertions |
| `test_als_rename_coalescing.py` | The rename heuristic, including its false positives |
| `test_als_golden.py` | Characterisation of the user-facing text and the CLI |
| `test_flp_parse.py` | Event widths, varint boundaries, the text-encoding heuristic |
| `test_robustness.py` | Malformed input: time-bounded, memory-bounded, no hangs |
| `test_storage_bench.py` | The shell benchmark: read-only, clean failures, delta wins |
| `test_repo_hygiene.py` | Rules from `AGENTS.md` that nothing else enforces |
| `test_real_fixtures.py` | Opt-in, on real material |

## Fixtures are generated, never committed

No audio and no DAW project file may enter this repository — `AGENTS.md`, `.gitignore`
and CI all say so, and `test_repo_hygiene.py` says so a third time so you find out in two
seconds instead of on a pull request.

So every fixture is built in code. `factories/als.py` emits gzipped XML with the element
paths verified against a real Live 12.3.5 set (`Mixer/Volume/Manual`,
`MainSequencer/Sample/ArrangerAutomation/Events/AudioClip`, `SampleRef/FileRef/RelativePath`
and so on), including realistic *noise* — warp markers, view state, absolute paths,
`OriginalCrc` — because a test that "only the view changed" is worthless if the fixture
has no view state to change. `factories/flp.py` emits exact bytes, which is what lets the
varint tests sit on 127/128 and 16383/16384 rather than near them.

The factories are verified by round-tripping through the real parsers
(`test_factories.py`). If those tests fail, do not trust anything else in the suite until
they pass.

## Adding a test

1. Build the input with a factory. Do not add a binary file to this directory.
2. Assert the **whole** output, not a substring. The value of a semantic diff is what it
   *doesn't* say; `assert "volume" in output` cannot catch new noise.
3. Isolate one change per test. `test_als_diff.py` is the pattern.
4. If you need real material, mark it `@pytest.mark.real_fixtures` and take the
   `real_material` fixture — never read a path from the environment yourself, or the
   loud-skip accounting breaks.
5. Found a bug in a prototype? Write the test that should pass, mark it
   `@pytest.mark.xfail(strict=True)`, and put the diagnosis *and the suggested fix* in
   the `reason`. `xfail_strict` is on, so the day someone fixes the bug the suite tells
   them to delete the marker. Do not fix a prototype and a test in the same commit
   without saying so — `AGENTS.md` treats a moving number as a finding.

## Known failures, on purpose

The suite has strict xfails, each pinning a real defect found while writing these tests.
They are documented in full in the `reason=` string next to each one. In short:

| Where | Defect |
|---|---|
| `als_semantic_diff.build_model` | Looks for `LiveSet/MasterTrack`; Live 12.3 writes `MainTrack`, so tempo is `None` on current Ableton and a tempo change is invisible |
| `als_semantic_diff.diff_models` | Rename detection never checks that the old sample disappeared, so swapped or partially-replaced samples are reported as file renames |
| `als_semantic_diff.diff_models` | Rename lines are ordered by set iteration, so output differs between processes (measured: 6 orderings from 6 `PYTHONHASHSEED` values) |
| `als_semantic_diff.build_model` | XML with no `<LiveSet>` dies with `AttributeError: 'NoneType'`; internal entity expansion is unbounded (218-byte file → 100 k characters); an 8.8 KB file can peak at 66 MB |
| `cdc_dedup.pairwise` | Counts each *distinct* chunk once against *every* byte of B, so a file compared with itself reports ~19 % reusable when it contains internal duplication |
| `flp_parse.decode_text` | Three-letter Latin-1 names decode to CJK mojibake; genuinely non-Latin UTF-16 names decode to Latin-1 mojibake |
| `flp_parse.parse` | Unbounded reads: a lying length field raises a bare `IndexError`; a long varint costs quadratic time (200 KB of `0xFF` → 3.6 s); the input file handle is never closed |

## What is deliberately not tested, and why

**Benchmark numbers.** No test asserts "delta chains are 29× smaller" or "FLAC keeps
47.2%". Those are measurements on specific real material, and `AGENTS.md` is explicit
that a ratio measured on synthetic data must not be presented as representative — music
is not white noise. The tests assert *mechanisms* (delta beats naive; reuse plus new
equals the whole file; chunk boundaries survive an insertion) and leave the numbers to
`docs/EXPERIMENTS.md`, where the material is named.

**Audio.** Nothing here reads or generates a `.wav`. Committing audio is forbidden;
synthesising it would produce exactly the misleading compression figures `AGENTS.md`
warns about. `experiments/null_diff.py`, which operates on renders, is therefore not
covered by this suite — it needs real audio and `ffmpeg`, and belongs with the real
fixture set once there is a way to supply one.

**DAW acceptance.** The most important untested thing in the project: whether Ableton
opens a file Wit wrote. `docs/EXPERIMENTS.md` open question 1 says so, and no test here
changes that. Nothing in this suite writes a `.als` intended for Live — the factory
produces files for *our* parser, and a passing suite is not evidence that Live would
open them.

**Merge.** `docs/EXPERIMENTS.md` 5 uses `git merge-file` by hand. There is no merge code
in `experiments/`, so there is nothing to test yet.

**Logic `ProjectData` structure.** `docs/FORMATS.md` documents the container, but no
prototype parses it, so only the storage path is exercised (via `cdc_dedup`) and only
when real material is supplied.

**Timing and throughput.** `cdc_dedup`'s own docstring says not to read timing numbers
off it. The one timing assertion in the suite is a *ratio* between two input sizes on the
same machine, used to detect quadratic behaviour, not a throughput figure.

## CI

CI currently does `py_compile`, `--help`, shellcheck, an audio-file grep, a repo-size
check and a docs link check. It does not run this suite. Wiring it up is two steps:

```yaml
- name: Tests
  run: |
    python3 -m pip install -r tests/requirements-dev.txt
    pytest tests/ --cov=experiments --cov-report=term-missing
```

Note that CI pins Python 3.9, which `test_repo_hygiene.py` also asserts the prototypes
still parse as. Real-fixture tests will skip there — that is expected, and the banner
will say so on every run.
