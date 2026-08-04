# CLAUDE.md

See [AGENTS.md](AGENTS.md) — it is the canonical brief for AI agents working on this
repository, and it applies to Claude Code as well.

Quick orientation:

- Wit versions the **recipe** (project structure + immutable source audio), never the
  **render**. Diffing rendered WAV bytes does not work and the measurements proving it
  are in [docs/EXPERIMENTS.md](docs/EXPERIMENTS.md).
- Never commit audio. Never modify the user's DAW projects — copy to scratch first.
- Every number in `docs/` must be reproducible. Label claims measured / cited / inferred.
