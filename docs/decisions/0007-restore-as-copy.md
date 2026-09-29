# ADR-0007: Restore a moment as a new copy, never in place

- **Status:** Accepted
- **Date:** 2026-09-29
- **Amends:** [ADR-0006](0006-consumer-surface-logic-first.md), decision 4 ("no project-file write-path")

## Context

ADR-0006 shipped a read-only slice: restore meant "reveal in Finder" or "copy with
caveats", and nothing in Wit wrote a project file. That kept the pilot safe, but it left
the producer's most concrete question unanswered: *"get me Tuesday's version back."*

A revealed backup doesn't answer it. A bare Logic `ProjectData` without the package's
shared `Media/` pool opens with missing-file errors (design review 1, wit-planning), which
is exactly the failure the read-only posture existed to prevent. A musician should get a
project that opens.

Sepehr decided on 2026-09-29 to bring restore into scope, **as a new copy only**.

## Decision

**Wit may write a restored project, but only as a new copy inside a dedicated Restores
folder. It never writes to, into, or over a project it watches.**

1. **Destination is fixed.** Restores go into one folder (`~/Music/Wit Restores` on macOS;
   the platform equivalent on Windows and Linux, per [ADR-0008](0008-cross-platform-pilot.md)).
   Each restore gets a fresh name (`<Song> — <date>`, then ` (2)`, …); an existing file
   or folder is never overwritten.
2. **The Restores folder is never inside a watched root**, never equal to one, and never
   a parent of one. This is checked on canonical, Unicode-normalised paths, with
   case-insensitive comparison where the volume is case-insensitive — never by string
   prefix.
3. **No write API takes a watched-project path.** Write functions accept only
   destinations derived from a validated Restores folder type, so the illegal call does
   not type-check. A property test covers symlinks, `..`, trailing separators, NFC/NFD
   and case variants, and Windows verbatim paths.
4. **Only bytes a DAW wrote.** A restore is assembled from files the DAW itself wrote
   (the current package, a stored `ProjectData` and `MetaData.plist`, a stored `.als` or
   `.flp`), never from bytes Wit synthesised:
   - **Logic / GarageBand:** clone the current package into the Restores folder (APFS
     clone where available, else a copy after a free-space check), swap in the stored
     `ProjectData` and `MetaData.plist` for that one alternative, and clear the copy's
     `Project File Backups`. Before writing, warn about audio the old save lists that
     no longer exists.
   - **Ableton / FL Studio:** write the stored `.als` / `.flp` into the Restores folder.
5. **A restore must be shown to open.** For each DAW, the restored copy is opened in that
   DAW and the result recorded in `docs/TESTING.md` §8 (no missing-media dialog, matching
   counts) before the pilot offers restore for that DAW.
6. **Restores are visible.** The watcher tags a restored copy "restored from <moment>", so
   it appears in the song's family rather than as an unexplained new project.

## Consequences

- ADR-0006's law changes from "Wit never writes a project file" to **"Wit never changes
  your projects"**: the originals stay untouched, and the only project files Wit writes
  are new copies in one folder the musician can see. The trust panel's sentence ("Wit
  never changes your projects") stays true.
- ROADMAP blocker #1 ("no Wit-produced file has ever been opened in a DAW") now applies
  to restore, per DAW: decision 5 is that gate. It still does not apply to the rest of
  the read-only slice.
- `wit-platform` holds the only code that writes outside Wit's own data folder. It is
  reviewed as safety-critical.
- The Logic swap is only as safe as "same package, older `ProjectData`". If Logic stores
  save-specific state elsewhere in the package that must match, restored copies may open
  with warnings; decision 5 exists to catch that before users do.

## What would overturn this

- A restored copy that fails to open, or opens with missing media, in a DAW where the
  gate had passed. Restore for that DAW goes back to reveal-only until the cause is
  understood.
- Any write that lands inside a watched root, in testing or in the field. That would
  mean decisions 2–3 failed, and restore is withdrawn until the guarantee is re-proven.
- Evidence that musicians don't use restore (pilot counters), in which case the write
  path isn't worth its risk and ADR-0006's read-only posture returns.

## Related

- [ADR-0006](0006-consumer-surface-logic-first.md): the read-only slice this amends.
- [ADR-0003](0003-plugin-state-policy.md): plugin state is copied as bytes, never
  interpreted, including in a restore.
- [ADR-0008](0008-cross-platform-pilot.md): where the Restores folder lives on each OS.
