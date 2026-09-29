# ADR-0008: The pilot runs on macOS, Windows and Linux, unsigned

- **Status:** Accepted
- **Date:** 2026-09-29
- **Amends:** [ADR-0006](0006-consumer-surface-logic-first.md) (macOS-only owner app)

## Context

ADR-0006 scoped the owner app to macOS, because the first cohort was Logic users and
Logic only runs on a Mac. Two things changed:

1. **The cohort is wider than Logic.** Sepehr wants the pilot to reach musician friends
   on any OS, not only Logic users on Macs, and the pilot plan asks for at least one
   friend each on Windows and Linux. A macOS-only app can't reach them.
2. **The UI is small.** The approved design (Shelf, Song view, Family, Send-ready check,
   Trust panel) is one web UI. Tauri 2 ships the same web UI on all three OSes; the
   per-OS work is in a thin platform layer (paths, watcher, tray, reveal, clone).

Sepehr decided on 2026-09-29 to pilot on all three OSes, and to stay unsigned on all of
them (declining Apple Developer ID enrollment again).

## Decision

**One Rust core, one web UI, thin platform shells; the pilot ships on macOS, Windows and
Linux; every build is unsigned; there is no updater.**

1. **Architecture.** The engine crates (`crates/*`) are platform-neutral. OS-specific
   behaviour lives in `crates/wit-platform`: discovery roots, the file watcher and its
   write-stability debounce, DAW-running detection, reveal in Finder / Explorer / the file
   manager, open with the DAW, clone-on-write copies, and path handling (NFC Unicode,
   Windows long paths, case-insensitive volumes). The app (`app/`, Tauri 2 + Svelte 5)
   is its own Cargo workspace and talks to the engine only through the Story contract
   (`crates/wit-story`).
2. **What each OS gets at pilot:**

   | OS | "What changed" in words | Every save kept + restore as copy | Bounce compare |
   |---|---|---|---|
   | macOS | Logic and GarageBand; Ableton; FL Studio (names; full on project format ≤ v24) | All of those, plus any DAW's project file | Yes |
   | Windows | Ableton; FL Studio (names; full on ≤ v24) | All of those, plus any DAW's project file | Yes |
   | Linux | None at pilot (stretch goal: REAPER `.rpp`, which is text) | Any DAW's project file | Yes |

   Linux's value at pilot is the generic History tier (every save kept and restorable)
   plus bounce compare. The app says so; it doesn't imply more.
3. **Unsigned everywhere.** macOS gets an ad-hoc-signed `.app` (Apple Silicon needs at
   least that to launch) and users approve it once in System Settings → Privacy &
   Security → *Open Anyway*. Windows users pass SmartScreen with *More info* → *Run
   anyway*. Linux users mark the AppImage executable. `PILOT.md` walks through each.
4. **No updater, no network.** The pilot has no auto-update, which keeps the "nothing
   leaves this computer" promise literal. New builds are sent by hand.
5. **CI proves all three.** The `rust` job runs on ubuntu, macOS and Windows. A separate
   app-build workflow produces the macOS `.app` zip (checked with `codesign -dv`), a
   Windows NSIS installer and a Linux AppImage/deb.

## Consequences

- ADR-0006's Logic-first *product* stance is unchanged: Logic is still where the richest
  sentences are. Only the "macOS-only surface" part is amended.
- Windows and Linux smoke tests run in VMs before the pilot; a "friend zero" install per
  OS happens before the hands-off week.
- Unsigned installs are real friction, and PILOT.md must describe the warnings honestly,
  not promise a workaround that doesn't exist.
- The tray needs AppIndicator on Linux (GNOME needs an extension); without a tray, Wit
  opens as a window.

## What would overturn this

- A pilot friend who can't get past an unsigned-app warning with PILOT.md alone. Then
  signing (Apple Developer ID; a Windows code-signing certificate) becomes a pilot
  prerequisite, not a 0.1 upgrade.
- Platform code growing past a thin layer (for example the UI needing per-OS forks). Then
  the "one UI" premise failed, and dropping an OS is cheaper than maintaining three apps.
- No pilot friends on Windows or Linux after all, in which case the extra surface costs
  more than it returns.

## Related

- [ADR-0006](0006-consumer-surface-logic-first.md): the consumer surface this widens.
- [ADR-0007](0007-restore-as-copy.md): restore-as-copy, whose Restores folder lives in a
  per-OS location defined by `wit-platform`.
- [ADR-0004](0004-implementation-stack.md): Rust core.
