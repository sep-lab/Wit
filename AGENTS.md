# AGENTS.md — instructions for AI coding agents working on Wit

This file is the canonical brief for any AI agent contributing to this repository.
`CLAUDE.md` points here. Read this before changing anything.

## What this project is

Wit is version control for music projects — "git for waves". It is **the version control
layer only**. It is not a hosting platform, not a DAW, and not a cross-DAW converter.
If a task seems to require building any of those, stop and ask.

## The one thing you must not get wrong

**Wit versions the recipe, not the render.**

Audio is already binary, and byte-level versioning of rendered audio does not work.
Measured: re-rendering a stem after a global EQ change leaves **0.00%** of bytes reusable,
because every sample value changes even though it sounds nearly identical. Git stores a
full new copy of a 20 MB stem per version.

So Wit versions the *project structure* (immutable source audio + the graph of
non-destructive edits) and treats renders/bounces/freezes as rebuildable cache.

Any proposal that starts with "we could just diff the WAV bytes" is already answered.
See [docs/decisions/0001-version-the-recipe-not-the-render.md](docs/decisions/0001-version-the-recipe-not-the-render.md).

## Rules for claims and numbers

This repository's credibility rests on its numbers being real.

- **Never invent a benchmark figure.** If you write a number in docs, you must have run
  the command that produced it, and you must say which material it was measured on.
- Label every claim as **measured**, **cited** (with URL), or **inferred**. Do not blur
  these. "Should be roughly" is inferred, not measured.
- Music is not white noise. Do not benchmark compression or dedup on synthetic audio and
  present it as representative. Sparse stems compress 100× better than dense masters —
  both are real, and a single number without material context is misleading.
- If you find an existing documented number is wrong, correct it and say so plainly in
  the PR. That is welcome, not embarrassing.

## Rules for handling user data

- **Never commit audio.** Not test fixtures, not "a small sample". `.gitignore` blocks
  common formats; do not work around it.
- **Never modify a user's DAW projects.** Read-only, always. Copy to a scratch directory
  before any experiment that writes.
- Sample and project files may contain other people's copyrighted material and personal
  paths. Real project files routinely embed absolute paths like
  `/Users/<someone-else>/Downloads/...`. Treat those as private data: do not paste them
  into issues or logs.

## Code conventions

- Prototypes in `experiments/` are Python 3.9+, **standard library only**, so a musician
  with a stock Mac can run them against their own session with no setup. Do not add
  dependencies there.
- Each experiment script must state, in its module docstring: what it measures, how to
  run it, and **what it does not handle**. Honest limitations are required, not optional.
- Prefer whitelist extraction over blacklist normalisation when modelling a DAW format.
  Blacklists leak: we tried excluding "churn" tags from Ableton XML and still got noisy
  results, while explicitly naming the fields we care about was clean.

## Things that are settled — do not relitigate without new evidence

| Decision | Where |
|---|---|
| Version the recipe, not the render | ADR-0001 |
| Content-addressed store with content-defined chunking | ADR-0002 |
| Plugin state is tracked opaquely, never interpreted or ported | ADR-0003 |
| Ableton is the first DAW target | ADR-0005 |

Each ADR lists what evidence would overturn it. Bring that evidence, or leave them alone.

## Testing

There is no product test suite yet. For now, a change to an experiment is validated by
re-running it on the fixture chain described in `experiments/README.md` and confirming
the documented numbers still reproduce. If a number moves, that is the finding — report
it, do not quietly update the doc.

## When to stop and ask

- The task requires launching a DAW, or would modify files outside this repository.
- The task requires uploading, hosting, or redistributing audio.
- You are about to make a claim you cannot measure.
- The change would expand scope toward a hosting platform or a cross-DAW converter.
