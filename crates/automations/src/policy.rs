//! Locked operational policies for scheduled automations, expressed as typed
//! behavior so the scheduler, persistence layer, and docs share one source.

use serde::{Deserialize, Serialize};

/// A due occurrence while a run of the same automation is still active is
/// skipped (recorded as `skipped_overlap`), never queued behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapPolicy {
    Skip,
}

pub const OVERLAP_POLICY: OverlapPolicy = OverlapPolicy::Skip;

/// What happens to an active run when its automation is edited, disabled, or
/// deleted: the run always finishes with its original prompt/context, and the
/// change applies from the next occurrence. Deletion additionally drops the
/// automation row once the active run releases its claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveEditPolicy {
    FinishThenApply,
}

pub const ACTIVE_EDIT_POLICY: ActiveEditPolicy = ActiveEditPolicy::FinishThenApply;

/// A failed run is recorded and the automation advances to its next
/// occurrence. No immediate retry, no automatic disabling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    AdvanceToNextOccurrence,
}

pub const FAILURE_POLICY: FailurePolicy = FailurePolicy::AdvanceToNextOccurrence;

/// At most this many different automations execute concurrently; further due
/// work waits its turn in the scheduler loop.
pub const MAX_CONCURRENT_RUNS: usize = 2;

/// Result summaries are truncated to this many characters before persistence.
pub const MAX_RESULT_SUMMARY_CHARS: usize = 2000;

/// Terminal runs kept per automation; older rows are pruned (audit log rows
/// are append-only and never pruned).
pub const RUN_HISTORY_RETAINED: usize = 50;

/// Truncate a result summary to the persistence cap, on a char boundary.
pub fn clamp_result_summary(s: &str) -> String {
    if s.chars().count() <= MAX_RESULT_SUMMARY_CHARS {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(MAX_RESULT_SUMMARY_CHARS - 1).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_preserves_short_and_truncates_long() {
        assert_eq!(
            clamp_result_summary("faktúra so splatnosťou"),
            "faktúra so splatnosťou"
        );
        let long = "á".repeat(MAX_RESULT_SUMMARY_CHARS + 10);
        let clamped = clamp_result_summary(&long);
        assert_eq!(clamped.chars().count(), MAX_RESULT_SUMMARY_CHARS);
        assert!(clamped.ends_with('…'));
    }
}
