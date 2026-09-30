//! Attendance cooldown.
//!
//! A student who has just been marked present must not be marked again within a
//! short window — they may linger in front of the kiosk, or re-approach. The
//! record is already written; a duplicate is noise in the data and a nuisance
//! on screen. This module is pure: it takes the last-marked time and "now".

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// Default cooldown window: 5 minutes, per Section 5.7 of the plan.
pub const DEFAULT_COOLDOWN_SECONDS: i64 = 300;

/// Whether a student may be marked present right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CooldownVerdict {
    /// Never marked, or the cooldown has elapsed — proceed to record.
    Allowed,
    /// Marked recently — show "sudah absen", do not record a second row.
    WithinCooldown {
        last_marked: DateTime<Utc>,
        remaining_seconds: i64,
    },
}

/// Evaluate the cooldown for one student.
///
/// `last_marked` is that student's most recent attendance timestamp on this
/// device (or globally, per policy) — `None` if never.
pub fn check(
    last_marked: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    window_seconds: i64,
) -> CooldownVerdict {
    let Some(last) = last_marked else {
        return CooldownVerdict::Allowed;
    };
    let elapsed = now.signed_duration_since(last);
    if elapsed >= Duration::seconds(window_seconds) {
        CooldownVerdict::Allowed
    } else {
        let remaining = (Duration::seconds(window_seconds) - elapsed)
            .num_seconds()
            .max(0);
        CooldownVerdict::WithinCooldown {
            last_marked: last,
            remaining_seconds: remaining,
        }
    }
}

/// A single student's attendance key. Distinct from [`Uuid`] for clarity.
pub type StudentId = Uuid;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn never_marked_is_allowed() {
        assert_eq!(
            check(None, at(0), DEFAULT_COOLDOWN_SECONDS),
            CooldownVerdict::Allowed
        );
    }

    #[test]
    fn just_marked_is_blocked() {
        let v = check(Some(at(0)), at(10), DEFAULT_COOLDOWN_SECONDS);
        match v {
            CooldownVerdict::WithinCooldown {
                remaining_seconds, ..
            } => {
                assert_eq!(remaining_seconds, 290);
            }
            CooldownVerdict::Allowed => panic!("expected cooldown block"),
        }
    }

    #[test]
    fn exactly_at_window_is_allowed() {
        assert_eq!(
            check(Some(at(0)), at(300), DEFAULT_COOLDOWN_SECONDS),
            CooldownVerdict::Allowed
        );
    }

    #[test]
    fn after_window_is_allowed() {
        assert_eq!(
            check(Some(at(0)), at(301), DEFAULT_COOLDOWN_SECONDS),
            CooldownVerdict::Allowed
        );
    }

    #[test]
    fn clock_skew_into_past_does_not_panic() {
        // now < last (clock moved backwards). Treat as within cooldown, no panic.
        let v = check(Some(at(100)), at(0), DEFAULT_COOLDOWN_SECONDS);
        assert!(matches!(v, CooldownVerdict::WithinCooldown { .. }));
    }
}
