# ADR-0004: Implementation stack — Rust core, Python for research

- **Status:** Accepted
- **Date:** 2026-08-05

## Context

Wit's core has an unusual mix of requirements:

- **CPU-bound work over gigabytes** — rolling-hash chunking, BLAKE3 hashing, zstd
  delta encoding, FLAC round-trips. A pure-Python chunker runs at 1–2 MB/s; a real one
  must sustain hundreds of MB/s or `wit status` on a 26 GB library is unusable.
- **Binary format parsing** — hostile, undocumented, byte-level. Needs precise control
  over endianness and offsets, and must never crash or over-allocate on malformed input
  (see [SECURITY.md](../../SECURITY.md)).
- **Cross-platform desktop distribution** — macOS and Windows, to musicians. Must be a
  single file they can run. Asking a producer to install a runtime loses most of them.
- **A background daemon** watching project folders, cheap enough to leave running while
  a DAW is using the CPU for audio.
- **A likely future plugin** (VST3/AU are C++ ABIs) and a likely future GUI.

## Decision

**Rust for the core engine and CLI. Python 3, standard library only, for research
prototypes in `experiments/`.**

### Why Rust

- Single statically-linked binary per platform; no runtime for the user to install.
- Best-in-class libraries for exactly this workload: `blake3`, `zstd`, `fastcdc`,
  `quick-xml`, `plist`, `flac-bound`.
- Memory safety matters here specifically: Wit parses adversarial binary files, and the
  entire class of "malformed `.ptx` corrupts memory" bugs disappears by construction.
- Predictable performance with no GC pauses — relevant for a daemon coexisting with a
  real-time audio process.
- Clean C ABI via `extern "C"`, so a future VST3/AU plugin shell in C++ can call the same
  core rather than reimplementing it.

### Why not the alternatives

- **Go** — genuinely close, and easier to hire for. Rejected on: weaker C++ interop for
  the plugin path, GC pauses next to a real-time audio thread, and a thinner ecosystem for
  audio codecs. If team velocity mattered more than these, Go would be the right call.
- **Python** — superb for prototyping (it produced every number in this repo) and wrong
  for shipping: too slow for GB-scale chunking, and desktop packaging for non-technical
  users is a persistent tax.
- **TypeScript/Node** — good for a future UI, wrong for the byte-level core.

### Prototypes stay Python

`experiments/` deliberately stays Python-with-no-dependencies so that a musician on a
stock Mac can run it against their own session immediately. Research code and production
code have different jobs; conflating them would slow both.

## Consequences

**Good**

- One binary, `wit`, that runs anywhere without a runtime.
- Fast enough to hash and chunk a whole library in the background.
- A clear FFI path to the DAW-plugin integration when it is needed.

**Bad, and accepted**

- Slower to write than Go or Python, and a smaller contributor pool. Mitigated by keeping
  the interesting exploratory work in Python where anyone can join.
- Rust's audio ecosystem is thinner than C++'s; some codec work may need FFI.
- Two languages in one repo is a real cost. It is justified only because the two have
  genuinely different requirements — and `experiments/` must never become a dependency of
  the shipped binary.

## What would overturn this

- Hitting sustained friction on DAW-plugin integration that Go or C++ would avoid.
- Discovering the CPU-bound path is not actually hot — if real libraries turn out small
  enough that Python is fast enough, the simplicity argument wins.

## Related

[ADR-0002](0002-storage-model.md), [CONTRIBUTING.md](../../CONTRIBUTING.md).
