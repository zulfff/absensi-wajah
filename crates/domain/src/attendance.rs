//! Attendance records and attempt audit types (Section 7 of the plan).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a recorded attendance row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttendanceStatus {
    /// Automatic recognition.
    Present,
    /// Manually recorded by a teacher (fallback path).
    Manual,
    /// Later corrected/voided by an admin. Kept for audit, excluded from counts.
    Corrected,
}

/// The outcome of a single kiosk interaction, logged for tuning (Section 5.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Accepted,
    RejectedUnknown,
    RejectedMargin,
    RejectedLiveness,
    RetryTimeout,
    CooldownShown,
    Error,
}

/// A recorded attendance event (not yet persisted).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttendanceRecord {
    pub student_id: Uuid,
    pub device_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub similarity: f32,
    pub margin: f32,
    pub liveness_score: f32,
    pub status: AttendanceStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serialises_snake_case() {
        assert_eq!(
            serde_json::to_string(&AttendanceStatus::Present).unwrap(),
            "\"present\""
        );
    }

    #[test]
    fn attempt_outcome_serialises_snake_case() {
        assert_eq!(
            serde_json::to_string(&AttemptOutcome::RejectedUnknown).unwrap(),
            "\"rejected_unknown\""
        );
    }
}
