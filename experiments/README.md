# Experiments

Research prototypes. These produced every number in [docs/EXPERIMENTS.md](../docs/EXPERIMENTS.md).

**Python 3.9+, standard library only** — no install step, so you can point them at your
own sessions immediately. That constraint is deliberate; do not add dependencies here.

| Script | What it does | Extra tools |
|---|---|---|
| `demo.py` | **Start here** — zero-input demo, no DAW required | — |
| `als_semantic_diff.py` | Ableton `.als` → a diff a musician can read | — |
| `cdc_dedup.py` | Content-defined chunking / dedup harness | — |
| `null_diff.py` | Audible diff between two renders — **any DAW, no parser** | `ffmpeg` |
| `flp_parse.py` | FL Studio `.flp` event-stream survey | — |
| `storage_bench.sh` | naive vs git vs delta chain | `zstd`, `git` |
| `reproduce_merge_daw_acceptance.py` | Reproduce the EXPERIMENTS.md §5 merge on your own project, for issue #1 (does Live open it?) | `git` |

## Try it on your own work

Ableton keeps timestamped autosaves in your project's `Backup/` folder, so **you already
have a version chain**:

```bash
python3 experiments/als_semantic_diff.py --chain '/path/to/YourProject/Backup/*.als'
./experiments/storage_bench.sh '/path/to/YourProject/Backup'
python3 experiments/flp_parse.py '/path/to/project.flp'
```

Reproducing the audio result needs two renders of the same stem, one with a global change
(any EQ or gain move across the whole track):

```bash
python3 experiments/cdc_dedup.py original.wav re_rendered.wav
```

Expect ~0% reuse. That result is the reason Wit versions project structure rather than
audio — see [ADR-0001](../docs/decisions/0001-version-the-recipe-not-the-render.md).

And the complement, which needs no format parser and therefore works for every DAW:

```bash
python3 experiments/null_diff.py old_bounce.wav new_bounce.wav
```

⚠️ Do not run a null test without alignment. A one-sample offset on *identical* audio
produces a residual that looks more different than any real edit (measured: −3.7 dB versus
−18.7 dB for a genuine localised change).

## Status of this code

Research instruments, not products. Held to a **lower** bar than shipped code — but their
**outputs** are held to a higher one, because design decisions rest on them.

Every script documents what it does *not* handle in its module docstring. Read that before
trusting a number. Known significant gap: `als_semantic_diff.py` models device *presence*
but not device *parameter values*, so it can under-report knob-only changes.

They are read-only and never modify your projects. Do not run them on untrusted files —
they are not hardened (see [SECURITY.md](../SECURITY.md)).

## Contributing a measurement

Numbers from other people's sessions are one of the most useful contributions right now —
ours come from a handful of projects on one machine. Open a **Measurement report** issue.
Results that contradict our published numbers are especially welcome.
