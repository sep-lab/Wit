//! Which DAWs are running, and how long each session lasted.
//!
//! # Guarantees
//!
//! - **Read-only.** [`RunningDaws::detect`] lists process names and
//!   executable paths through `sysinfo`; it never signals, inspects the
//!   memory of, or launches a process.
//! - **Name matching is pure and unit-tested** ([`identify_process`]), so
//!   no test depends on a real DAW being installed or running.
//! - **Session bookkeeping is pure** ([`SessionTracker`]): it is fed
//!   snapshots and timestamps and returns open/quit events; it never reads
//!   the clock itself.
//!
//! # Process names matched
//!
//! Names come from the executable's file name on macOS (`sysinfo` uses
//! `proc_pidpath`), `/proc/<pid>/comm` on Linux (truncated to 15 bytes by
//! the kernel), and the image name on Windows. Matching is ASCII
//! case-insensitive and ignores spaces, `-`, `_`, and a trailing `.exe`.
//!
//! | DAW | Names |
//! |---|---|
//! | Logic Pro | `Logic Pro`, `Logic Pro X`, `Logic Pro Creator Studio` |
//! | GarageBand | `GarageBand` |
//! | Ableton Live | `Ableton Live <edition>…` (Windows), or `Live` when the executable sits inside an `Ableton…` app bundle / folder (macOS) |
//! | FL Studio | `FL Studio…`, `FL64.exe`, `FL.exe`, `OsxFL` |
//! | REAPER | `REAPER`, `reaper`, `reaper64` |
//! | Bitwig Studio | `Bitwig Studio`, `BitwigStudio`, `bitwig-studio` |
//!
//! These names are **inferred** from each vendor's shipping bundle layout,
//! not captured from every version; a DAW whose process is named
//! differently shows as "not running", never as a different DAW.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// The DAWs Wit knows by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Daw {
    LogicPro,
    GarageBand,
    AbletonLive,
    FlStudio,
    Reaper,
    BitwigStudio,
}

impl Daw {
    pub const ALL: [Daw; 6] = [
        Daw::LogicPro,
        Daw::GarageBand,
        Daw::AbletonLive,
        Daw::FlStudio,
        Daw::Reaper,
        Daw::BitwigStudio,
    ];

    pub const fn display_name(self) -> &'static str {
        match self {
            Daw::LogicPro => "Logic Pro",
            Daw::GarageBand => "GarageBand",
            Daw::AbletonLive => "Ableton Live",
            Daw::FlStudio => "FL Studio",
            Daw::Reaper => "REAPER",
            Daw::BitwigStudio => "Bitwig Studio",
        }
    }
}

/// Lowercase, drop a trailing `.exe`, and remove spaces, `-`, `_`.
fn compact(name: &str) -> (String, bool) {
    let lower = name.trim().to_ascii_lowercase();
    let (base, was_exe) = match lower.strip_suffix(".exe") {
        Some(b) => (b.to_string(), true),
        None => (lower, false),
    };
    (
        base.chars()
            .filter(|c| !matches!(c, ' ' | '-' | '_'))
            .collect(),
        was_exe,
    )
}

/// Identify a DAW from a process name and, when known, its executable path.
pub fn identify_process(name: &OsStr, exe: Option<&Path>) -> Option<Daw> {
    let name = name.to_string_lossy();
    let (c, was_exe) = compact(&name);
    let exe_mentions = |needle: &str| {
        exe.is_some_and(|p| {
            p.to_string_lossy()
                .to_ascii_lowercase()
                .replace([' ', '-', '_'], "")
                .contains(needle)
        })
    };
    match c.as_str() {
        "logicpro" | "logicprox" | "logicprocreatorstudio" => Some(Daw::LogicPro),
        "garageband" => Some(Daw::GarageBand),
        // macOS: `Ableton Live 12 Suite.app/Contents/MacOS/Live`. "Live" alone
        // is too generic to trust without the bundle path.
        "live" if exe_mentions("ableton") => Some(Daw::AbletonLive),
        _ if c.starts_with("abletonlive") => Some(Daw::AbletonLive),
        "fl64" | "fl" if was_exe || exe_mentions("imageline") || exe_mentions("flstudio") => {
            Some(Daw::FlStudio)
        }
        "osxfl" => Some(Daw::FlStudio),
        _ if c.starts_with("flstudio") => Some(Daw::FlStudio),
        "reaper" | "reaper64" => Some(Daw::Reaper),
        _ if c.starts_with("bitwigstudio") => Some(Daw::BitwigStudio),
        _ => None,
    }
}

/// The DAWs running at one instant, with their process ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunningDaws {
    pids: BTreeMap<Daw, BTreeSet<u32>>,
}

impl RunningDaws {
    /// Snapshot the real process table (names and executable paths only).
    pub fn detect() -> RunningDaws {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
        );
        RunningDaws::from_processes(
            system
                .processes()
                .iter()
                .map(|(pid, p)| (pid.as_u32(), p.name(), p.exe())),
        )
    }

    /// Build a snapshot from `(pid, name, exe)` triples — the pure half of
    /// [`RunningDaws::detect`].
    pub fn from_processes<'a>(
        processes: impl IntoIterator<Item = (u32, &'a OsStr, Option<&'a Path>)>,
    ) -> RunningDaws {
        let mut pids: BTreeMap<Daw, BTreeSet<u32>> = BTreeMap::new();
        for (pid, name, exe) in processes {
            if let Some(daw) = identify_process(name, exe) {
                pids.entry(daw).or_default().insert(pid);
            }
        }
        RunningDaws { pids }
    }

    pub fn contains(&self, daw: Daw) -> bool {
        self.pids.contains_key(&daw)
    }

    pub fn is_empty(&self) -> bool {
        self.pids.is_empty()
    }

    pub fn daws(&self) -> impl Iterator<Item = Daw> + '_ {
        self.pids.keys().copied()
    }

    pub fn pids(&self, daw: Daw) -> impl Iterator<Item = u32> + '_ {
        self.pids.get(&daw).into_iter().flatten().copied()
    }
}

/// A DAW appearing in, or disappearing from, successive snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    Opened {
        daw: Daw,
        at: SystemTime,
    },
    Quit {
        daw: Daw,
        opened_at: SystemTime,
        quit_at: SystemTime,
    },
}

impl SessionEvent {
    /// Session length for a `Quit` (zero if the clock went backwards).
    pub fn duration(&self) -> Option<Duration> {
        match self {
            SessionEvent::Opened { .. } => None,
            SessionEvent::Quit {
                opened_at, quit_at, ..
            } => Some(quit_at.duration_since(*opened_at).unwrap_or_default()),
        }
    }
}

/// Turns a stream of [`RunningDaws`] snapshots into open/quit events.
///
/// The first snapshot opens a session for every DAW already running, timed
/// from that snapshot (Wit can't know when it really started).
#[derive(Debug, Clone, Default)]
pub struct SessionTracker {
    open: BTreeMap<Daw, SystemTime>,
}

impl SessionTracker {
    pub fn new() -> SessionTracker {
        SessionTracker::default()
    }

    /// Feed one snapshot taken at `now`.
    pub fn observe(&mut self, snapshot: &RunningDaws, now: SystemTime) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        let quit: Vec<Daw> = self
            .open
            .keys()
            .copied()
            .filter(|d| !snapshot.contains(*d))
            .collect();
        for daw in quit {
            if let Some(opened_at) = self.open.remove(&daw) {
                events.push(SessionEvent::Quit {
                    daw,
                    opened_at,
                    quit_at: now,
                });
            }
        }
        for daw in snapshot.daws() {
            if let std::collections::btree_map::Entry::Vacant(slot) = self.open.entry(daw) {
                slot.insert(now);
                events.push(SessionEvent::Opened { daw, at: now });
            }
        }
        events
    }

    /// DAWs with a session open, and since when.
    pub fn open_sessions(&self) -> impl Iterator<Item = (Daw, SystemTime)> + '_ {
        self.open.iter().map(|(d, t)| (*d, *t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str, exe: Option<&str>) -> Option<Daw> {
        identify_process(OsStr::new(name), exe.map(Path::new))
    }

    #[test]
    fn macos_bundle_executables() {
        assert_eq!(id("Logic Pro", None), Some(Daw::LogicPro));
        assert_eq!(id("Logic Pro X", None), Some(Daw::LogicPro));
        assert_eq!(id("Logic Pro Creator Studio", None), Some(Daw::LogicPro));
        assert_eq!(id("GarageBand", None), Some(Daw::GarageBand));
        assert_eq!(
            id(
                "Live",
                Some("/Applications/Ableton Live 12 Suite.app/Contents/MacOS/Live")
            ),
            Some(Daw::AbletonLive)
        );
        assert_eq!(id("OsxFL", None), Some(Daw::FlStudio));
        assert_eq!(id("FL Studio", None), Some(Daw::FlStudio));
        assert_eq!(id("REAPER", None), Some(Daw::Reaper));
        assert_eq!(id("BitwigStudio", None), Some(Daw::BitwigStudio));
    }

    #[test]
    fn windows_image_names() {
        assert_eq!(
            id("Ableton Live 12 Suite.exe", None),
            Some(Daw::AbletonLive)
        );
        assert_eq!(id("Ableton Live 11 Lite.exe", None), Some(Daw::AbletonLive));
        assert_eq!(id("FL64.exe", None), Some(Daw::FlStudio));
        assert_eq!(id("FL.exe", None), Some(Daw::FlStudio));
        assert_eq!(id("reaper.exe", None), Some(Daw::Reaper));
        assert_eq!(id("Bitwig Studio.exe", None), Some(Daw::BitwigStudio));
    }

    #[test]
    fn linux_comm_names() {
        assert_eq!(id("reaper", None), Some(Daw::Reaper));
        assert_eq!(id("bitwig-studio", None), Some(Daw::BitwigStudio));
    }

    #[test]
    fn near_misses_are_not_daws() {
        // Too generic without evidence it's Ableton's bundle.
        assert_eq!(id("Live", None), None);
        assert_eq!(id("Live", Some("/usr/bin/Live")), None);
        assert_eq!(id("fl", None), None);
        assert_eq!(id("Logic Pro Helper", None), None);
        assert_eq!(id("Ableton Index.exe", None), None);
        assert_eq!(id("flatpak", None), None);
        assert_eq!(id("reaperd", None), None);
        assert_eq!(id("Finder", None), None);
    }

    #[test]
    fn snapshots_group_pids_by_daw() {
        let snap = RunningDaws::from_processes([
            (10, OsStr::new("Logic Pro"), None),
            (11, OsStr::new("Finder"), None),
            (12, OsStr::new("reaper"), None),
            (13, OsStr::new("reaper"), None),
        ]);
        assert!(snap.contains(Daw::LogicPro));
        assert!(!snap.contains(Daw::GarageBand));
        assert_eq!(snap.pids(Daw::Reaper).collect::<Vec<_>>(), vec![12, 13]);
        assert_eq!(snap.daws().count(), 2);
    }

    #[test]
    fn detect_runs_without_panicking() {
        // No assertion about *which* DAWs run on a CI box — only that the
        // real process table can be read.
        let _ = RunningDaws::detect();
    }

    #[test]
    fn session_tracker_reports_open_and_quit_with_duration() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let t1 = t0 + Duration::from_secs(60);
        let t2 = t1 + Duration::from_secs(3_600);
        let logic = RunningDaws::from_processes([(1, OsStr::new("Logic Pro"), None)]);
        let none = RunningDaws::default();
        let mut tracker = SessionTracker::new();

        assert!(tracker.observe(&none, t0).is_empty());
        assert_eq!(
            tracker.observe(&logic, t1),
            vec![SessionEvent::Opened {
                daw: Daw::LogicPro,
                at: t1
            }]
        );
        assert!(tracker
            .observe(&logic, t1 + Duration::from_secs(5))
            .is_empty());
        let quit = tracker.observe(&none, t2);
        assert_eq!(quit.len(), 1);
        assert_eq!(quit[0].duration(), Some(Duration::from_secs(3_600)));
        assert_eq!(tracker.open_sessions().count(), 0);
    }
}
