//! Schedule semantics: typed recurrence rules, validation, and deterministic
//! next-occurrence calculation. Pure logic — no persistence, HTTP, or UI.
//!
//! Policies (locked):
//! - Ambiguous local times (DST fall-back) resolve to the **earlier** instant.
//! - Nonexistent local times (DST spring-forward) advance to the **next valid
//!   local time** on the same day.
//! - Missed runs: at most **one** catch-up execution, and only if the missed
//!   occurrence is within the last 24 hours; otherwise skip and advance.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Minimum recurrence interval: one hour.
pub const MIN_INTERVAL_HOURS: u32 = 1;
/// Maximum recurrence interval: one year, guards nonsense input.
pub const MAX_INTERVAL_HOURS: u32 = 24 * 366;
/// Missed occurrences older than this are skipped, never caught up.
pub const CATCH_UP_WINDOW: Duration = Duration::hours(24);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScheduleError {
    #[error("name must not be empty")]
    EmptyName,
    #[error("prompt must not be empty")]
    EmptyPrompt,
    #[error("invalid IANA time zone: {0}")]
    InvalidTimeZone(String),
    #[error("interval must be between {MIN_INTERVAL_HOURS} and {MAX_INTERVAL_HOURS} hours")]
    InvalidInterval,
    #[error("selected weekdays must not be empty")]
    EmptyWeekdays,
    #[error("schedule has no next occurrence")]
    NoNextOccurrence,
}

/// Wire-stable weekday (lowercase in JSON so the Swift client encodes it directly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    pub fn to_chrono(self) -> chrono::Weekday {
        match self {
            Weekday::Mon => chrono::Weekday::Mon,
            Weekday::Tue => chrono::Weekday::Tue,
            Weekday::Wed => chrono::Weekday::Wed,
            Weekday::Thu => chrono::Weekday::Thu,
            Weekday::Fri => chrono::Weekday::Fri,
            Weekday::Sat => chrono::Weekday::Sat,
            Weekday::Sun => chrono::Weekday::Sun,
        }
    }

    pub const WEEKDAYS: [Weekday; 5] = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
    ];
}

/// How an automation repeats. `time` values are local times in the
/// automation's IANA time zone; recurrence is always calculated in that zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecurrenceRule {
    EveryNHours { hours: u32 },
    Daily { time: NaiveTime },
    Weekdays { time: NaiveTime },
    SelectedWeekdays { days: Vec<Weekday>, time: NaiveTime },
    Weekly { day: Weekday, time: NaiveTime },
}

/// Complete schedule: a one-shot instant or a recurrence rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutomationSchedule {
    /// Run once at the given UTC instant.
    Once { at: DateTime<Utc> },
    Recurring { rule: RecurrenceRule },
}

/// Parse and validate an IANA time-zone identifier.
pub fn parse_timezone(tz: &str) -> Result<Tz, ScheduleError> {
    Tz::from_str(tz).map_err(|_| ScheduleError::InvalidTimeZone(tz.to_string()))
}

impl RecurrenceRule {
    pub fn validate(&self) -> Result<(), ScheduleError> {
        match self {
            RecurrenceRule::EveryNHours { hours } => {
                if *hours < MIN_INTERVAL_HOURS || *hours > MAX_INTERVAL_HOURS {
                    return Err(ScheduleError::InvalidInterval);
                }
            }
            RecurrenceRule::SelectedWeekdays { days, .. } => {
                if days.is_empty() {
                    return Err(ScheduleError::EmptyWeekdays);
                }
            }
            RecurrenceRule::Daily { .. }
            | RecurrenceRule::Weekdays { .. }
            | RecurrenceRule::Weekly { .. } => {}
        }
        Ok(())
    }

    fn matches_weekday(&self, day: chrono::Weekday) -> bool {
        match self {
            RecurrenceRule::EveryNHours { .. } | RecurrenceRule::Daily { .. } => true,
            RecurrenceRule::Weekdays { .. } => Weekday::WEEKDAYS
                .iter()
                .any(|w| w.to_chrono() == day),
            RecurrenceRule::SelectedWeekdays { days, .. } => {
                days.iter().any(|w| w.to_chrono() == day)
            }
            RecurrenceRule::Weekly { day: d, .. } => d.to_chrono() == day,
        }
    }

    fn local_time(&self) -> Option<NaiveTime> {
        match self {
            RecurrenceRule::EveryNHours { .. } => None,
            RecurrenceRule::Daily { time }
            | RecurrenceRule::Weekdays { time }
            | RecurrenceRule::SelectedWeekdays { time, .. }
            | RecurrenceRule::Weekly { time, .. } => Some(*time),
        }
    }
}

impl AutomationSchedule {
    pub fn validate(&self, tz: &str) -> Result<(), ScheduleError> {
        parse_timezone(tz)?;
        match self {
            AutomationSchedule::Once { .. } => Ok(()),
            AutomationSchedule::Recurring { rule } => rule.validate(),
        }
    }

    /// First occurrence strictly after `after`, or `None` (one-shot in the past).
    pub fn next_occurrence(
        &self,
        tz: Tz,
        after: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        match self {
            AutomationSchedule::Once { at } => (*at > after).then_some(*at),
            AutomationSchedule::Recurring { rule } => match rule {
                // Fixed-duration interval: DST-agnostic, computed in UTC.
                RecurrenceRule::EveryNHours { hours } => {
                    Some(after + Duration::hours(*hours as i64))
                }
                _ => next_local_occurrence(rule, tz, after),
            },
        }
    }
}

/// Walk forward day by day in the automation's local zone until the rule's
/// weekday matches and the resolved instant is strictly after `after`.
fn next_local_occurrence(
    rule: &RecurrenceRule,
    tz: Tz,
    after: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let time = rule.local_time().expect("local-time rules only");
    let after_local = after.with_timezone(&tz);
    let mut date = after_local.date_naive();
    // 8 days covers every weekly pattern plus a same-day miss.
    for _ in 0..9 {
        if rule.matches_weekday(date.weekday()) {
            if let Some(instant) = resolve_local(tz, date, time) {
                if instant > after {
                    return Some(instant);
                }
            }
        }
        date = date.succ_opt()?;
    }
    None
}

/// Resolve a local wall-clock datetime to a UTC instant using the locked DST
/// policies: ambiguous → earlier instant; nonexistent → next valid local time.
pub fn resolve_local(tz: Tz, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    let naive = date.and_time(time);
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(earlier, _) => Some(earlier.with_timezone(&Utc)),
        LocalResult::None => {
            // Spring-forward gap: scan forward minute by minute (gaps are ≤ 2 h).
            let mut candidate = naive;
            for _ in 0..180 {
                candidate += Duration::minutes(1);
                match tz.from_local_datetime(&candidate) {
                    LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => {
                        return Some(dt.with_timezone(&Utc));
                    }
                    LocalResult::None => continue,
                }
            }
            None
        }
    }
}

/// What to do with an occurrence whose scheduled time is already in the past.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissedRunDecision {
    /// Within the 24 h window: run one catch-up execution now.
    CatchUp,
    /// Older than the window: record as skipped and advance to the next occurrence.
    SkipStale,
}

pub fn missed_run_decision(scheduled: DateTime<Utc>, now: DateTime<Utc>) -> MissedRunDecision {
    if now - scheduled <= CATCH_UP_WINDOW {
        MissedRunDecision::CatchUp
    } else {
        MissedRunDecision::SkipStale
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    const BA: &str = "Europe/Bratislava";

    fn tz() -> Tz {
        parse_timezone(BA).unwrap()
    }

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn t(s: &str) -> NaiveTime {
        s.parse().unwrap()
    }

    #[test]
    fn once_in_future_fires_once() {
        let s = AutomationSchedule::Once { at: utc("2026-07-20T07:30:00Z") };
        assert_eq!(
            s.next_occurrence(tz(), utc("2026-07-19T00:00:00Z")),
            Some(utc("2026-07-20T07:30:00Z"))
        );
        // After it has passed there is no next occurrence.
        assert_eq!(s.next_occurrence(tz(), utc("2026-07-20T07:30:00Z")), None);
    }

    #[test]
    fn every_n_hours_is_fixed_duration() {
        let s = AutomationSchedule::Recurring { rule: RecurrenceRule::EveryNHours { hours: 2 } };
        assert_eq!(
            s.next_occurrence(tz(), utc("2026-07-16T10:15:00Z")),
            Some(utc("2026-07-16T12:15:00Z"))
        );
    }

    #[test]
    fn daily_at_local_time() {
        // 08:00 Bratislava summer = 06:00 UTC.
        let s = AutomationSchedule::Recurring { rule: RecurrenceRule::Daily { time: t("08:00:00") } };
        assert_eq!(
            s.next_occurrence(tz(), utc("2026-07-16T05:00:00Z")),
            Some(utc("2026-07-16T06:00:00Z"))
        );
        // Already past 08:00 local → tomorrow.
        assert_eq!(
            s.next_occurrence(tz(), utc("2026-07-16T06:00:00Z")),
            Some(utc("2026-07-17T06:00:00Z"))
        );
    }

    #[test]
    fn weekdays_skip_weekend() {
        // 2026-07-17 is a Friday.
        let s = AutomationSchedule::Recurring { rule: RecurrenceRule::Weekdays { time: t("08:00:00") } };
        assert_eq!(
            s.next_occurrence(tz(), utc("2026-07-17T06:00:00Z")), // Fri 08:00 local exactly
            Some(utc("2026-07-20T06:00:00Z"))                     // Mon
        );
    }

    #[test]
    fn selected_weekdays() {
        let s = AutomationSchedule::Recurring {
            rule: RecurrenceRule::SelectedWeekdays {
                days: vec![Weekday::Tue, Weekday::Sat],
                time: t("09:00:00"),
            },
        };
        // From Fri 2026-07-17 → Sat 2026-07-18 09:00 local (07:00 UTC).
        assert_eq!(
            s.next_occurrence(tz(), utc("2026-07-17T12:00:00Z")),
            Some(utc("2026-07-18T07:00:00Z"))
        );
    }

    #[test]
    fn weekly_same_day_time_passed_goes_next_week() {
        // 2026-07-17 is a Friday; 07:45 local = 05:45 UTC.
        let s = AutomationSchedule::Recurring {
            rule: RecurrenceRule::Weekly { day: Weekday::Fri, time: t("07:45:00") },
        };
        assert_eq!(
            s.next_occurrence(tz(), utc("2026-07-17T05:45:00Z")),
            Some(utc("2026-07-24T05:45:00Z"))
        );
    }

    #[test]
    fn spring_forward_nonexistent_time_advances_to_next_valid() {
        // Bratislava 2026-03-29: 02:00 → 03:00, so 02:30 local does not exist.
        // Policy: next valid local time = 03:00 CEST = 01:00 UTC.
        let s = AutomationSchedule::Recurring { rule: RecurrenceRule::Daily { time: t("02:30:00") } };
        assert_eq!(
            s.next_occurrence(tz(), utc("2026-03-28T23:00:00Z")),
            Some(utc("2026-03-29T01:00:00Z"))
        );
    }

    #[test]
    fn fall_back_ambiguous_time_picks_earlier_instant() {
        // Bratislava 2026-10-25: 03:00 → 02:00, so 02:30 local happens twice.
        // Policy: earlier instant = 02:30 CEST = 00:30 UTC.
        let s = AutomationSchedule::Recurring { rule: RecurrenceRule::Daily { time: t("02:30:00") } };
        assert_eq!(
            s.next_occurrence(tz(), utc("2026-10-24T23:00:00Z")),
            Some(utc("2026-10-25T00:30:00Z"))
        );
    }

    #[test]
    fn daily_recurrence_is_local_not_utc_plus_24h() {
        // Across the fall-back transition a daily 08:00 automation moves from
        // 06:00 UTC (CEST) to 07:00 UTC (CET) — not a fixed +24 h.
        let s = AutomationSchedule::Recurring { rule: RecurrenceRule::Daily { time: t("08:00:00") } };
        let sat = s.next_occurrence(tz(), utc("2026-10-24T00:00:00Z")).unwrap();
        let sun = s.next_occurrence(tz(), sat).unwrap();
        assert_eq!(sat, utc("2026-10-24T06:00:00Z"));
        assert_eq!(sun, utc("2026-10-25T07:00:00Z"));
    }

    #[test]
    fn interval_validation() {
        assert_eq!(
            RecurrenceRule::EveryNHours { hours: 0 }.validate(),
            Err(ScheduleError::InvalidInterval)
        );
        assert_eq!(
            RecurrenceRule::EveryNHours { hours: MAX_INTERVAL_HOURS + 1 }.validate(),
            Err(ScheduleError::InvalidInterval)
        );
        assert!(RecurrenceRule::EveryNHours { hours: 1 }.validate().is_ok());
    }

    #[test]
    fn empty_selected_weekdays_rejected() {
        assert_eq!(
            RecurrenceRule::SelectedWeekdays { days: vec![], time: t("08:00:00") }.validate(),
            Err(ScheduleError::EmptyWeekdays)
        );
    }

    #[test]
    fn invalid_timezone_rejected() {
        assert!(matches!(
            parse_timezone("Mars/OlympusMons"),
            Err(ScheduleError::InvalidTimeZone(_))
        ));
        // Fixed offsets are not IANA identifiers.
        assert!(parse_timezone("UTC+2").is_err());
        assert!(parse_timezone(BA).is_ok());
    }

    #[test]
    fn catch_up_window() {
        let now = utc("2026-07-17T12:00:00Z");
        assert_eq!(
            missed_run_decision(utc("2026-07-17T00:00:00Z"), now),
            MissedRunDecision::CatchUp
        );
        assert_eq!(
            missed_run_decision(utc("2026-07-15T12:00:00Z"), now),
            MissedRunDecision::SkipStale
        );
    }

    #[test]
    fn weekday_json_is_lowercase() {
        assert_eq!(serde_json::to_string(&Weekday::Mon).unwrap(), "\"mon\"");
        let rule: RecurrenceRule = serde_json::from_str(
            r#"{"type":"selected_weekdays","days":["mon","fri"],"time":"08:00:00"}"#,
        )
        .unwrap();
        assert!(rule.validate().is_ok());
        // Malformed weekday fails at parse time.
        assert!(serde_json::from_str::<RecurrenceRule>(
            r#"{"type":"selected_weekdays","days":["monday"],"time":"08:00:00"}"#
        )
        .is_err());
    }
}
