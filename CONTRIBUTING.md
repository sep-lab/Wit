# Contributing to Wit

Thanks for being here. Wit is early — the design is still moving, which means a
well-argued issue can change the project more than a patch can.

You do not need to be a systems programmer to help. If you have shipped a session to a
collaborator and it went badly, you have information this project needs.

## Get oriented in 10 minutes

Read in this order. Each answers the question the next one assumes.

1. **[docs/DECISION-DOC.md](docs/DECISION-DOC.md)** — why this project exists, what was
   decided, and what is still unknown. Start here.
2. **[docs/EXPERIMENTS.md](docs/EXPERIMENTS.md)** — the evidence. Every number, its
   method, and its limits.
3. **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** — how it fits together, and which of
   git's ideas we take and reject.
4. **[docs/PRIOR-ART.md](docs/PRIOR-ART.md)** — the ~20 products that tried this and
   died, why, and what we are doing differently. Read this before deciding to invest
   time; you should know the risk you are signing up for.
5. **[docs/FORMATS.md](docs/FORMATS.md)** — what each DAW writes to disk. Read when you
   pick up format work.

The one idea everything rests on: **Wit versions the recipe (project structure + immutable
source audio), not the render.** Rendered audio cannot be versioned at the byte level — a
global EQ change leaves 0% of bytes reusable. If a proposal starts with "just diff the WAV
bytes", that is already answered in
[ADR-0001](docs/decisions/0001-version-the-recipe-not-the-render.md).

## Set up and run something real

No install step. Python 3.9+, standard library only.

```bash
git clone https://github.com/sep-lab/Wit.git && cd Wit
python3 experiments/demo.py          # see the whole idea, no DAW needed
```

Now point it at your own work. Ableton keeps timestamped autosaves in your project's
`Backup/` folder, so **you already have a version chain to test against**:

```bash
python3 experiments/als_semantic_diff.py --chain '/path/to/YourProject/Backup/*.als'
```

Optional, for the storage benchmark: `brew install zstd` (or your package manager).
Optional, to reproduce the audio experiments: `ffmpeg` and `flac`.

Everything in `experiments/` is **read-only and never modifies your projects**. If you
write a new one, keep it that way — copy to a temp directory before any experiment that
writes.

## Ways to contribute that we especially want

**1. Format reverse-engineering.** The hardest, most valuable work. If you can map
another chunk of Logic's `ProjectData`, or document an FL Studio event ID, that directly
unblocks DAW support. See [docs/FORMATS.md](docs/FORMATS.md) for what is already known
and what is still unknown.

**2. Test fixtures.** We need real project files across DAW versions — ideally the *same*
project saved repeatedly, since that is what exercises diffing.

> ⚠️ **Only contribute sessions you own outright and that contain no third-party
> copyrighted samples.** Do not upload commercial sample packs. A project that
> references samples by name without shipping the audio is usually fine and often more
> useful. Never commit audio to this repo — see `.gitignore`.

**3. Workflow reports.** Write up how your studio actually collaborates: who sends what
to whom, in what format, and where it breaks. Open an issue with the
`workflow-report` label. This shapes the roadmap more than feature requests do.

**4. Adversarial review.** If you think a claim in `docs/` is wrong, please say so — with
a counter-measurement if you can. Several of the design's load-bearing numbers came from
testing an assumption and finding it false. That is the point.

## Ground rules for claims

Wit's documentation makes quantitative claims. They must be reproducible.

- If you state a number, say how you measured it and on what material.
- Distinguish **measured** from **cited** from **inferred**. Use those words.
- "Should be fine" is not a measurement. Neither is a benchmark on synthetic audio —
  music is not white noise and does not compress like it.

If you cannot measure something yet, write down the experiment that would settle it. See
the open questions at the end of [docs/DECISION-DOC.md](docs/DECISION-DOC.md).

## About the prototypes

Each script in `experiments/` documents its own usage and states what it does *not*
handle. They are research instruments, not products — held to a lower bar than shipped
code, but their **outputs** are held to a higher one, because design decisions rest on
them. Read the module docstring before trusting a number.

The production implementation language is tracked in
[ADR-0004](docs/decisions/0004-implementation-stack.md).

## Pull requests

1. Open an issue first for anything non-trivial, so we can agree on the approach.
2. Keep PRs focused. A format-parsing change and a docs restructure should be separate.
3. Update the relevant doc in the same PR. Undocumented behaviour is a bug here.
4. If you change a number in `docs/`, include the command that produced it.

Commit messages: short imperative subject, and explain *why* in the body when it is not
obvious.

```
Add warp marker extraction to the Ableton model

Warp markers are 5,112 of the elements in a typical set, so omitting them
made "no musical change" diffs unreliable on time-stretched material.
```

## Reporting bugs

Include your OS, DAW and exact DAW version, and — if the DAW rejected a file Wit
produced — say so explicitly. That is the most serious class of bug this project can
have, and it will be prioritised over everything else.

Never attach audio or a full project to an issue. Describe it, or share a minimal
synthetic reproduction.

## Security

Please do not open public issues for security problems. See [SECURITY.md](SECURITY.md).

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). Be decent to people.

## License

Contributions are licensed under [Apache 2.0](LICENSE), matching the project.
