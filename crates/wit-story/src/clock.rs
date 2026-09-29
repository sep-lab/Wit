//! Calendar labels without a date-time dependency: "Thu 23:05",
//! "Mon · night · about 1h 40m". Pure functions of a Unix timestamp and the
//! viewer's UTC offset, so the same input always renders the same label
//! (the fixture test depends on it).

use crate::types::Timestamp;
use std::sync::Arc;

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The viewer's UTC offset in minutes. `PerInstant` lets the caller answer
/// per timestamp, so a save from before a daylight-saving switch is labelled
/// with the offset that applied then.
#[derive(Clone)]
pub enum Offset {
    Fixed(i32),
    PerInstant(Arc<dyn Fn(Timestamp) -> i32 + Send + Sync>),
}

impl std::fmt::Debug for Offset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Offset::Fixed(m) => write!(f, "Offset::Fixed({m})"),
            Offset::PerInstant(_) => write!(f, "Offset::PerInstant(..)"),
        }
    }
}

/// The viewer's clock: "now" and the UTC offset, both passed in by the
/// caller (the app reads them from the OS; tests fix them).
#[derive(Debug, Clone)]
pub struct Clock {
    pub now: Timestamp,
    pub offset: Offset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Civil {
    year: i64,
    month: u32, // 1..=12
    day: u32,   // 1..=31
    weekday: usize,
    hour: u32,
    minute: u32,
    /// Days since the epoch in local time, for "today" and "this week".
    local_day: i64,
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

impl Clock {
    /// A clock with one fixed UTC offset for every timestamp.
    pub fn fixed(now: Timestamp, utc_offset_minutes: i32) -> Self {
        Clock {
            now,
            offset: Offset::Fixed(utc_offset_minutes),
        }
    }

    fn offset_minutes(&self, t: Timestamp) -> i32 {
        match &self.offset {
            Offset::Fixed(m) => *m,
            Offset::PerInstant(f) => f(t),
        }
    }

    fn civil(&self, t: Timestamp) -> Civil {
        let local = t.0 + i64::from(self.offset_minutes(t)) * 60;
        let local_day = local.div_euclid(86_400);
        let secs = local.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(local_day);
        Civil {
            year,
            month,
            day,
            // 1970-01-01 was a Thursday.
            weekday: (local_day + 4).rem_euclid(7) as usize,
            hour: (secs / 3600) as u32,
            minute: ((secs % 3600) / 60) as u32,
            local_day,
        }
    }

    /// "Today", "Yesterday", "Thu", "21 Sep", or "21 Sep 2025" depending on
    /// how long ago.
    pub fn day_label(&self, t: Timestamp) -> String {
        let c = self.civil(t);
        let today = self.civil(self.now);
        match today.local_day - c.local_day {
            0 => "Today".to_string(),
            1 => "Yesterday".to_string(),
            2..=6 => WEEKDAYS[c.weekday].to_string(),
            _ if c.year == today.year => format!("{} {}", c.day, MONTHS[(c.month - 1) as usize]),
            _ => format!("{} {} {}", c.day, MONTHS[(c.month - 1) as usize], c.year),
        }
    }

    /// "23:05".
    pub fn time_label(&self, t: Timestamp) -> String {
        let c = self.civil(t);
        format!("{:02}:{:02}", c.hour, c.minute)
    }

    /// "Thu 23:05".
    pub fn moment_label(&self, t: Timestamp) -> String {
        format!("{} {}", self.day_label(t), self.time_label(t))
    }

    /// "morning", "afternoon", "evening", "night", "late night".
    pub fn part_of_day(&self, t: Timestamp) -> &'static str {
        match self.civil(t).hour {
            5..=11 => "morning",
            12..=16 => "afternoon",
            17..=20 => "evening",
            21..=23 | 0..=1 => "night",
            _ => "late night",
        }
    }
}

/// "about 1h 40m", rounded to 5 minutes. `None` under 5 minutes: a session
/// that short says nothing worth an "about".
pub fn about_duration(secs: u64) -> Option<String> {
    let minutes = ((secs + 150) / 300) * 5;
    if minutes < 5 {
        return None;
    }
    let (h, m) = (minutes / 60, minutes % 60);
    Some(match (h, m) {
        (0, m) => format!("about {m}m"),
        (h, 0) => format!("about {h}h"),
        (h, m) => format!("about {h}h {m}m"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock_at(now: i64) -> Clock {
        Clock::fixed(Timestamp(now), 0)
    }

    #[test]
    fn epoch_is_a_thursday() {
        // Two days later, so the label is the weekday.
        assert_eq!(clock_at(2 * 86_400).moment_label(Timestamp(0)), "Thu 00:00");
    }

    #[test]
    fn labels_go_today_yesterday_weekday_date() {
        // 2026-09-24 23:05 UTC is a Thursday.
        let t = 1_790_291_100;
        assert_eq!(clock_at(t + 60).moment_label(Timestamp(t)), "Today 23:05");
        assert_eq!(
            clock_at(t + 86_400).moment_label(Timestamp(t)),
            "Yesterday 23:05"
        );
        assert_eq!(
            clock_at(t + 3 * 86_400).moment_label(Timestamp(t)),
            "Thu 23:05"
        );
        assert_eq!(
            clock_at(t + 8 * 86_400).moment_label(Timestamp(t)),
            "24 Sep 23:05"
        );
        assert_eq!(
            clock_at(t + 400 * 86_400).day_label(Timestamp(t)),
            "24 Sep 2026"
        );
    }

    #[test]
    fn utc_offset_moves_the_day() {
        let t = 1_790_291_100; // Thu 23:05 UTC
        let tehran = Clock::fixed(Timestamp(t + 3 * 86_400), 210);
        assert_eq!(tehran.moment_label(Timestamp(t)), "Fri 02:35");
    }

    #[test]
    fn a_per_instant_offset_labels_each_save_with_its_own_offset() {
        // A made-up switch: +60 before t0, +120 from t0 on.
        let t0 = 1_790_000_000;
        let clock = Clock {
            now: Timestamp(t0 + 5 * 86_400),
            offset: Offset::PerInstant(Arc::new(
                move |t: Timestamp| {
                    if t.0 < t0 {
                        60
                    } else {
                        120
                    }
                },
            )),
        };
        let before = clock.time_label(Timestamp(t0 - 3600));
        let after = clock.time_label(Timestamp(t0 + 3600));
        // Two saves two hours apart in UTC show three hours apart locally.
        // t0 is 14:13:20 UTC: 13:13 UTC + 1h = 14:13; 15:13 UTC + 2h = 17:13.
        assert_eq!((before.as_str(), after.as_str()), ("14:13", "17:13"));
    }

    #[test]
    fn durations_round_to_five_minutes_and_drop_tiny_ones() {
        assert_eq!(about_duration(60), None);
        assert_eq!(about_duration(6_000).as_deref(), Some("about 1h 40m"));
        assert_eq!(about_duration(3_600).as_deref(), Some("about 1h"));
        assert_eq!(about_duration(1_500).as_deref(), Some("about 25m"));
    }
}
