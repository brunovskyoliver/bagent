//! Automation domain model shared by the scheduler, persistence, API, and UI.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::schedule::{AutomationSchedule, ScheduleError};

pub const MAX_NAME_CHARS: usize = 80;
pub const MAX_PROMPT_CHARS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AutomationId(pub Uuid);

impl AutomationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AutomationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AutomationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AutomationRunId(pub Uuid);

impl AutomationRunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AutomationRunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AutomationRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    pub id: AutomationId,
    /// Monotonic revision captured into each immutable Task Snapshot.
    pub definition_revision: i64,
    pub name: String,
    /// Natural-language task prompt — user-authored, never a policy override.
    pub prompt: String,
    pub enabled: bool,
    /// IANA time-zone identifier, e.g. "Europe/Bratislava".
    pub timezone: String,
    pub schedule: AutomationSchedule,
    /// Persisted UTC instant of the next execution; `None` when exhausted.
    pub next_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_run_status: Option<AutomationRunStatus>,
    /// Concise latest-result summary suitable for the notch.
    pub last_result_summary: Option<String>,
}

impl Automation {
    /// Validate user-editable fields. The backend is authoritative — the UI
    /// never decides validity.
    pub fn validate(
        name: &str,
        prompt: &str,
        schedule: &AutomationSchedule,
        tz: &str,
    ) -> Result<(), ScheduleError> {
        if name.trim().is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(ScheduleError::EmptyName);
        }
        if prompt.trim().is_empty() || prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(ScheduleError::EmptyPrompt);
        }
        schedule.validate(tz)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunStatus {
    Running,
    Completed,
    /// Finished, but at least one step was blocked (approval denied/timed out)
    /// or a non-fatal tool failure occurred.
    Partial,
    Failed,
    /// A due occurrence was skipped because a run of the same automation was
    /// still active.
    SkippedOverlap,
    /// A missed occurrence outside the catch-up window was intentionally skipped.
    SkippedStale,
    /// Found unfinished after a daemon restart.
    Abandoned,
}

impl AutomationRunStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, AutomationRunStatus::Running)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AutomationRunStatus::Running => "running",
            AutomationRunStatus::Completed => "completed",
            AutomationRunStatus::Partial => "partial",
            AutomationRunStatus::Failed => "failed",
            AutomationRunStatus::SkippedOverlap => "skipped_overlap",
            AutomationRunStatus::SkippedStale => "skipped_stale",
            AutomationRunStatus::Abandoned => "abandoned",
        }
    }
}

impl std::str::FromStr for AutomationRunStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "running" => AutomationRunStatus::Running,
            "completed" => AutomationRunStatus::Completed,
            "partial" => AutomationRunStatus::Partial,
            "failed" => AutomationRunStatus::Failed,
            "skipped_overlap" => AutomationRunStatus::SkippedOverlap,
            "skipped_stale" => AutomationRunStatus::SkippedStale,
            "abandoned" => AutomationRunStatus::Abandoned,
            other => return Err(format!("unknown run status: {other}")),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationRun {
    pub id: AutomationRunId,
    pub automation_id: AutomationId,
    /// The occurrence this run satisfies (UTC).
    pub scheduled_for: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: AutomationRunStatus,
    /// Concise, redacted final result for display. Never raw tool payloads.
    pub result_summary: Option<String>,
    pub is_catch_up: bool,
    /// True for `run-now`, false for scheduler-claimed runs.
    pub is_manual: bool,
}

/// Trusted execution context handed to the agent runtime for every scheduled
/// run. Safety guarantees live here — not in the natural-language prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationExecutionContext {
    pub automation_id: AutomationId,
    pub automation_name: String,
    pub run_id: AutomationRunId,
    pub scheduled_for: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    pub is_catch_up: bool,
    /// Always true for scheduled/run-now executions: no human is watching, so
    /// gated writes must go through a fresh pending approval.
    pub unattended: bool,
    pub timezone: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::{AutomationSchedule, RecurrenceRule, ScheduleError};

    fn schedule() -> AutomationSchedule {
        AutomationSchedule::Recurring {
            rule: RecurrenceRule::EveryNHours { hours: 2 },
        }
    }

    #[test]
    fn validate_rejects_empty_fields_and_bad_tz() {
        assert_eq!(
            Automation::validate("", "do it", &schedule(), "Europe/Bratislava"),
            Err(ScheduleError::EmptyName)
        );
        assert_eq!(
            Automation::validate("Mail check", "  ", &schedule(), "Europe/Bratislava"),
            Err(ScheduleError::EmptyPrompt)
        );
        assert!(matches!(
            Automation::validate("Mail check", "do it", &schedule(), "Nope/Nope"),
            Err(ScheduleError::InvalidTimeZone(_))
        ));
        assert!(
            Automation::validate("Mail check", "do it", &schedule(), "Europe/Bratislava").is_ok()
        );
    }

    #[test]
    fn run_status_round_trips() {
        for s in [
            AutomationRunStatus::Running,
            AutomationRunStatus::Completed,
            AutomationRunStatus::Partial,
            AutomationRunStatus::Failed,
            AutomationRunStatus::SkippedOverlap,
            AutomationRunStatus::SkippedStale,
            AutomationRunStatus::Abandoned,
        ] {
            assert_eq!(s.as_str().parse::<AutomationRunStatus>().unwrap(), s);
        }
        assert!(!AutomationRunStatus::Running.is_terminal());
        assert!(AutomationRunStatus::Failed.is_terminal());
    }
}
