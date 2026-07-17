//! Daemon-owned automation scheduler.
//!
//! Design: persisted `next_run_at` timestamps + one Tokio task that sleeps
//! until the earliest due instant and is woken immediately through
//! `AppState.automations_changed` whenever an automation changes or a run
//! finishes. Sleep is chunked at 60 s because macOS monotonic timers do not
//! advance during system sleep — each chunk re-reads the wall clock, so
//! sleep/wake and clock changes recalculate within a minute without
//! short-interval polling.
//!
//! Missed-run policy (locked in bagent-automations): at most one catch-up per
//! automation, and only within 24 h of the missed instant; older occurrences
//! are recorded as `skipped_stale` and the schedule advances. Overlap policy:
//! a due occurrence while the same automation is running is recorded as
//! `skipped_overlap`. Failures are persisted and never retried immediately —
//! the automation simply waits for its next occurrence.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rusqlite::{params, Connection};
use serde_json::json;

use bagent_automations::{
    missed_run_decision, parse_timezone, Automation, AutomationRun, MissedRunDecision,
};

use crate::automations_api::{repo_claim_run, repo_get, repo_insert_run, RepoError};
use crate::AppState;

/// Considered a catch-up when the scheduled instant is further in the past
/// than this (restart / wake), as opposed to normal dispatch latency.
const CATCH_UP_LATENCY: ChronoDuration = ChronoDuration::minutes(5);
/// Maximum single timer chunk — bounds wake-up latency after system sleep.
const MAX_SLEEP_CHUNK_SECS: u64 = 60;
/// Idle sleep when nothing is scheduled (Notify still wakes immediately).
const IDLE_SLEEP_SECS: u64 = 3600;

fn audit(conn: &Connection, action: &str, meta: serde_json::Value) {
    let _ = conn.execute(
        "INSERT INTO audit_entries (action, payload, model) VALUES (?1, ?2, '')",
        params![action, meta.to_string()],
    );
}

/// Advance an automation past `now` (NULL for exhausted one-shots).
fn advance_next_run(
    conn: &Connection,
    a: &Automation,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let next = parse_timezone(&a.timezone)
        .ok()
        .and_then(|tz| a.schedule.next_occurrence(tz, now));
    let _ = conn.execute(
        "UPDATE automations SET next_run_at=?2, updated_at=?3 WHERE id=?1",
        params![
            a.id.to_string(),
            next.map(|t| t.to_rfc3339()),
            now.to_rfc3339()
        ],
    );
    next
}

fn record_skip(
    conn: &Connection,
    a: &Automation,
    scheduled_for: DateTime<Utc>,
    status: bagent_automations::AutomationRunStatus,
    now: DateTime<Utc>,
) {
    let run = AutomationRun {
        id: bagent_automations::AutomationRunId::new(),
        automation_id: a.id,
        scheduled_for,
        started_at: None,
        finished_at: Some(now),
        status,
        result_summary: None,
        is_catch_up: false,
        is_manual: false,
    };
    let _ = repo_insert_run(conn, &run);
}

/// One synchronous scheduling pass: claim every due automation, record
/// stale/overlap skips, and advance schedules. Deterministic — `now` is
/// injected. Returns the claimed work to execute (no DB locks held after).
pub(crate) fn scheduler_step(
    conn: &Connection,
    now: DateTime<Utc>,
) -> Vec<(Automation, AutomationRun)> {
    let due_ids: Vec<String> = {
        let mut stmt = match conn.prepare(
            "SELECT id FROM automations WHERE enabled=1 AND next_run_at IS NOT NULL \
             AND next_run_at <= ?1 ORDER BY next_run_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![now.to_rfc3339()], |r| r.get(0))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    };

    let mut claimed = Vec::new();
    for id in due_ids {
        let Ok(a) = repo_get(conn, &id) else { continue };
        let Some(scheduled_for) = a.next_run_at else {
            continue;
        };

        match missed_run_decision(scheduled_for, now) {
            MissedRunDecision::SkipStale => {
                record_skip(
                    conn,
                    &a,
                    scheduled_for,
                    bagent_automations::AutomationRunStatus::SkippedStale,
                    now,
                );
                audit(
                    conn,
                    "automation_run_skipped_stale",
                    json!({
                        "automation_id": id, "scheduled_for": scheduled_for.to_rfc3339(),
                    }),
                );
                advance_next_run(conn, &a, now);
            }
            MissedRunDecision::CatchUp => {
                let is_catch_up = now - scheduled_for > CATCH_UP_LATENCY;
                match repo_claim_run(conn, &a, scheduled_for, is_catch_up, false, now) {
                    Ok(run) => {
                        if is_catch_up {
                            audit(
                                conn,
                                "automation_run_catch_up",
                                json!({
                                    "automation_id": id, "scheduled_for": scheduled_for.to_rfc3339(),
                                }),
                            );
                        }
                        advance_next_run(conn, &a, now);
                        claimed.push((a, run));
                    }
                    Err(RepoError::ActiveRun) => {
                        record_skip(
                            conn,
                            &a,
                            scheduled_for,
                            bagent_automations::AutomationRunStatus::SkippedOverlap,
                            now,
                        );
                        audit(
                            conn,
                            "automation_run_overlap_skip",
                            json!({
                                "automation_id": id, "scheduled_for": scheduled_for.to_rfc3339(),
                            }),
                        );
                        advance_next_run(conn, &a, now);
                    }
                    Err(e) => {
                        tracing::error!("scheduler claim failed for {id}: {e:?}");
                    }
                }
            }
        }
    }
    claimed
}

/// Startup recovery: abandoned `running` rows become terminal `abandoned`
/// (they never revive), their automations advance normally on the next pass,
/// and stale undecided approvals are denied — approvals are never revived
/// across a restart.
pub(crate) fn recover_on_startup(conn: &Connection, now: DateTime<Utc>) -> usize {
    let nows = now.to_rfc3339();
    let abandoned = conn
        .execute(
            "UPDATE automation_runs SET status='abandoned', finished_at=?1, \
             result_summary='Daemon restarted during execution.' WHERE status='running'",
            params![nows],
        )
        .unwrap_or(0);
    if abandoned > 0 {
        audit(
            conn,
            "automation_runs_abandoned",
            json!({"count": abandoned}),
        );
        let _ = conn.execute(
            "UPDATE automations SET last_run_status='abandoned', last_run_at=?1 \
             WHERE id IN (SELECT automation_id FROM automation_runs WHERE status='abandoned' AND finished_at=?1)",
            params![nows],
        );
    }
    let denied = conn
        .execute(
            "UPDATE pending_approvals SET decision='deny', decided_at=?1 WHERE decision IS NULL",
            params![nows],
        )
        .unwrap_or(0);
    if denied > 0 {
        audit(
            conn,
            "approvals_denied_on_restart",
            json!({"count": denied}),
        );
    }
    abandoned
}

/// Earliest enabled due instant, for sleep computation.
fn next_due_at(conn: &Connection) -> Option<DateTime<Utc>> {
    conn.query_row(
        "SELECT MIN(next_run_at) FROM automations WHERE enabled=1 AND next_run_at IS NOT NULL",
        [],
        |r| r.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
    .map(|d| d.with_timezone(&Utc))
}

/// The scheduler task. Runs until the daemon exits; a run in flight at
/// shutdown is recovered as `abandoned` on the next start.
pub(crate) async fn run_scheduler(state: AppState) {
    {
        let conn = state.db.lock().await;
        let recovered = recover_on_startup(&conn, Utc::now());
        if recovered > 0 {
            tracing::info!("scheduler: recovered {recovered} abandoned run(s)");
        }
    }
    loop {
        let claimed = {
            let conn = state.db.lock().await;
            scheduler_step(&conn, Utc::now())
        };
        for (automation, run) in claimed {
            state.publish_event(json!({
                "type": "automation_next_run_changed",
                "automation_id": automation.id.to_string(),
            }));
            let st = state.clone();
            tokio::spawn(async move {
                // Bounded concurrency: at most policy::MAX_CONCURRENT_RUNS
                // executions at once (shared with run-now).
                crate::automations_api::execute_automation_run(st, automation, run).await;
            });
        }

        let wait_secs = {
            let conn = state.db.lock().await;
            match next_due_at(&conn) {
                Some(t) => {
                    let until = (t - Utc::now()).num_seconds().max(0) as u64;
                    until.clamp(1, MAX_SLEEP_CHUNK_SECS)
                }
                None => IDLE_SLEEP_SECS,
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(wait_secs)) => {}
            _ = state.automations_changed.notified() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automations_api::{repo_create, repo_recent_runs, repo_set_enabled};
    use bagent_automations::{AutomationRunStatus, AutomationSchedule};
    use chrono::TimeZone;

    fn test_conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::embedded::migrations::runner()
            .run(&mut conn)
            .unwrap();
        conn
    }

    fn t(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn once(
        conn: &Connection,
        name: &str,
        at: DateTime<Utc>,
        created: DateTime<Utc>,
    ) -> Automation {
        repo_create(
            conn,
            name,
            "check unread mail",
            "Europe/Bratislava",
            &AutomationSchedule::Once { at },
            true,
            created,
        )
        .unwrap()
    }

    #[test]
    fn due_one_shot_is_claimed_and_exhausted() {
        let conn = test_conn();
        let a = once(&conn, "n", t(2026, 7, 18, 6, 0), t(2026, 7, 17, 0, 0));
        // Not due yet.
        assert!(scheduler_step(&conn, t(2026, 7, 18, 5, 59)).is_empty());
        // Due: claimed exactly once, next_run_at cleared.
        let claimed = scheduler_step(&conn, t(2026, 7, 18, 6, 0));
        assert_eq!(claimed.len(), 1);
        assert!(!claimed[0].1.is_catch_up); // on-time dispatch
        let after = repo_get(&conn, &a.id.to_string()).unwrap();
        assert_eq!(after.next_run_at, None);
        // Second pass claims nothing.
        assert!(scheduler_step(&conn, t(2026, 7, 18, 6, 1)).is_empty());
    }

    #[test]
    fn missed_within_window_is_single_catch_up() {
        let conn = test_conn();
        let a = once(&conn, "n", t(2026, 7, 18, 6, 0), t(2026, 7, 17, 0, 0));
        // Daemon was down; wakes 3 hours later → one catch-up flagged run.
        let claimed = scheduler_step(&conn, t(2026, 7, 18, 9, 0));
        assert_eq!(claimed.len(), 1);
        assert!(claimed[0].1.is_catch_up);
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            None
        );
    }

    #[test]
    fn stale_one_shot_is_skipped_not_executed() {
        let conn = test_conn();
        let a = once(&conn, "n", t(2026, 7, 18, 6, 0), t(2026, 7, 17, 0, 0));
        // Wakes 3 days later → intentionally skipped, recorded, exhausted.
        let claimed = scheduler_step(&conn, t(2026, 7, 21, 6, 0));
        assert!(claimed.is_empty());
        let runs = repo_recent_runs(&conn, &a.id.to_string(), 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, AutomationRunStatus::SkippedStale);
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            None
        );
    }

    #[test]
    fn overlap_is_recorded_and_skipped() {
        let conn = test_conn();
        let a = once(&conn, "n", t(2026, 7, 18, 6, 0), t(2026, 7, 17, 0, 0));
        // A manual run is still active when the scheduled instant arrives.
        repo_claim_run(
            &conn,
            &a,
            t(2026, 7, 18, 5, 0),
            false,
            true,
            t(2026, 7, 18, 5, 0),
        )
        .unwrap();
        let claimed = scheduler_step(&conn, t(2026, 7, 18, 6, 0));
        assert!(claimed.is_empty());
        let runs = repo_recent_runs(&conn, &a.id.to_string(), 10).unwrap();
        assert!(runs
            .iter()
            .any(|r| r.status == AutomationRunStatus::SkippedOverlap));
    }

    #[test]
    fn disabled_automations_never_fire() {
        let conn = test_conn();
        let a = once(&conn, "n", t(2026, 7, 18, 6, 0), t(2026, 7, 17, 0, 0));
        repo_set_enabled(&conn, &a.id.to_string(), false, t(2026, 7, 17, 1, 0)).unwrap();
        assert!(scheduler_step(&conn, t(2026, 7, 18, 6, 0)).is_empty());
    }

    #[test]
    fn restart_recovery_abandons_runs_and_denies_stale_approvals() {
        let conn = test_conn();
        let a = once(&conn, "n", t(2026, 7, 18, 6, 0), t(2026, 7, 17, 0, 0));
        let run = repo_claim_run(
            &conn,
            &a,
            t(2026, 7, 18, 6, 0),
            false,
            false,
            t(2026, 7, 18, 6, 0),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pending_approvals (id, tool_name, description, expires_at, created_at) \
             VALUES ('ap1', 'whatsapp.send_message', 'x', '2099-01-01T00:00:00Z', '2026-07-18T06:00:00Z')",
            [],
        )
        .unwrap();

        let recovered = recover_on_startup(&conn, t(2026, 7, 18, 6, 30));
        assert_eq!(recovered, 1);
        let runs = repo_recent_runs(&conn, &a.id.to_string(), 10).unwrap();
        assert_eq!(runs[0].status, AutomationRunStatus::Abandoned);
        assert_eq!(runs[0].id, run.id);
        let decision: Option<String> = conn
            .query_row(
                "SELECT decision FROM pending_approvals WHERE id='ap1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(decision.as_deref(), Some("deny"));
        // The abandoned run releases the claim: the automation can run again.
        assert!(repo_claim_run(
            &conn,
            &a,
            t(2026, 7, 18, 7, 0),
            true,
            false,
            t(2026, 7, 18, 7, 0)
        )
        .is_ok());
    }

    #[test]
    fn daily_recurrence_advances_in_local_zone_across_dst() {
        use bagent_automations::RecurrenceRule;
        let conn = test_conn();
        // Daily 08:00 Bratislava, created the day before the fall-back change.
        let a = repo_create(
            &conn,
            "daily",
            "p",
            "Europe/Bratislava",
            &AutomationSchedule::Recurring {
                rule: RecurrenceRule::Daily {
                    time: "08:00:00".parse().unwrap(),
                },
            },
            true,
            t(2026, 10, 23, 12, 0),
        )
        .unwrap();
        // Sat 2026-10-24: 08:00 CEST = 06:00 UTC.
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            Some(t(2026, 10, 24, 6, 0))
        );
        let claimed = scheduler_step(&conn, t(2026, 10, 24, 6, 0));
        assert_eq!(claimed.len(), 1);
        // Advanced to Sun 2026-10-25: 08:00 CET = 07:00 UTC — local time, not +24h UTC.
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            Some(t(2026, 10, 25, 7, 0))
        );
        // No catch-up loop: an immediate second pass claims nothing.
        assert!(scheduler_step(&conn, t(2026, 10, 24, 6, 1)).is_empty());
    }

    #[test]
    fn every_n_hours_advances_and_failure_does_not_retry() {
        use bagent_automations::RecurrenceRule;
        let conn = test_conn();
        let a = repo_create(
            &conn,
            "hourly",
            "p",
            "Europe/Bratislava",
            &AutomationSchedule::Recurring {
                rule: RecurrenceRule::EveryNHours { hours: 2 },
            },
            true,
            t(2026, 7, 17, 10, 0),
        )
        .unwrap();
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            Some(t(2026, 7, 17, 12, 0))
        );
        let claimed = scheduler_step(&conn, t(2026, 7, 17, 12, 0));
        assert_eq!(claimed.len(), 1);
        // Advanced two hours past the dispatch instant.
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            Some(t(2026, 7, 17, 14, 0))
        );
        // The run fails — outcome persists, schedule is NOT pulled earlier.
        crate::automations_api::repo_finish_run(
            &conn,
            &claimed[0].1.id.to_string(),
            &a.id.to_string(),
            AutomationRunStatus::Failed,
            Some("Model error"),
            t(2026, 7, 17, 12, 1),
        )
        .unwrap();
        let after = repo_get(&conn, &a.id.to_string()).unwrap();
        assert_eq!(after.last_run_status, Some(AutomationRunStatus::Failed));
        assert_eq!(after.next_run_at, Some(t(2026, 7, 17, 14, 0)));
        assert!(scheduler_step(&conn, t(2026, 7, 17, 12, 2)).is_empty());
    }

    #[test]
    fn recurring_missed_beyond_window_skips_then_advances() {
        use bagent_automations::RecurrenceRule;
        let conn = test_conn();
        let a = repo_create(
            &conn,
            "daily",
            "p",
            "Europe/Bratislava",
            &AutomationSchedule::Recurring {
                rule: RecurrenceRule::Daily {
                    time: "08:00:00".parse().unwrap(),
                },
            },
            true,
            t(2026, 7, 17, 0, 0),
        )
        .unwrap();
        // Laptop closed for a week: exactly one skip record, schedule jumps to
        // the next future occurrence — no replay of the missed week.
        let claimed = scheduler_step(&conn, t(2026, 7, 25, 12, 0));
        assert!(claimed.is_empty());
        let runs = repo_recent_runs(&conn, &a.id.to_string(), 20).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, AutomationRunStatus::SkippedStale);
        // Next occurrence: tomorrow 08:00 local (06:00 UTC in July).
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            Some(t(2026, 7, 26, 6, 0))
        );
    }

    #[test]
    fn weekday_recurrence_skips_weekend_and_handles_spring_forward() {
        use bagent_automations::RecurrenceRule;
        let conn = test_conn();
        // Weekdays 08:00 Bratislava; created Fri 2026-03-27 (spring-forward on
        // Sun 2026-03-29, 02:00→03:00).
        let a = repo_create(
            &conn,
            "weekdays",
            "p",
            "Europe/Bratislava",
            &AutomationSchedule::Recurring {
                rule: RecurrenceRule::Weekdays {
                    time: "08:00:00".parse().unwrap(),
                },
            },
            true,
            t(2026, 3, 27, 6, 0),
        )
        .unwrap();
        // Fri 08:00 CET = 07:00 UTC.
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            Some(t(2026, 3, 27, 7, 0))
        );
        let claimed = scheduler_step(&conn, t(2026, 3, 27, 7, 0));
        assert_eq!(claimed.len(), 1);
        // Weekend skipped, and Monday is CEST: 08:00 = 06:00 UTC.
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            Some(t(2026, 3, 30, 6, 0))
        );
    }

    #[test]
    fn weekly_recurrence_catch_up_and_overlap() {
        use bagent_automations::{RecurrenceRule, Weekday};
        let conn = test_conn();
        // Weekly Friday 07:45 Bratislava (2026-07-17 is a Friday; 05:45 UTC).
        let a = repo_create(
            &conn,
            "weekly",
            "p",
            "Europe/Bratislava",
            &AutomationSchedule::Recurring {
                rule: RecurrenceRule::Weekly {
                    day: Weekday::Fri,
                    time: "07:45:00".parse().unwrap(),
                },
            },
            true,
            t(2026, 7, 16, 0, 0),
        )
        .unwrap();
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            Some(t(2026, 7, 17, 5, 45))
        );

        // Wakes 4 hours late (restart): one catch-up, advanced to next Friday.
        let claimed = scheduler_step(&conn, t(2026, 7, 17, 9, 45));
        assert_eq!(claimed.len(), 1);
        assert!(claimed[0].1.is_catch_up);
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            Some(t(2026, 7, 24, 5, 45))
        );

        // Next Friday arrives while the catch-up run is still active → overlap
        // skip recorded, schedule advances again.
        let claimed2 = scheduler_step(&conn, t(2026, 7, 24, 5, 45));
        assert!(claimed2.is_empty());
        let runs = repo_recent_runs(&conn, &a.id.to_string(), 10).unwrap();
        assert!(runs
            .iter()
            .any(|r| r.status == AutomationRunStatus::SkippedOverlap));
        assert_eq!(
            repo_get(&conn, &a.id.to_string()).unwrap().next_run_at,
            Some(t(2026, 7, 31, 5, 45))
        );
    }

    #[test]
    fn malformed_weekday_and_empty_selection_rejected_at_api_boundary() {
        // Wire-level rejection: unknown weekday fails deserialization, empty
        // selected set fails validation before persistence.
        assert!(serde_json::from_str::<bagent_automations::RecurrenceRule>(
            r#"{"type":"weekly","day":"friday","time":"07:45:00"}"#
        )
        .is_err());
        let empty: bagent_automations::RecurrenceRule =
            serde_json::from_str(r#"{"type":"selected_weekdays","days":[],"time":"07:45:00"}"#)
                .unwrap();
        assert!(empty.validate().is_err());
    }

    #[test]
    fn edit_disable_delete_during_active_run_is_finish_then_apply() {
        use crate::automations_api::{repo_delete, repo_update, AutomationPatch, RepoError};
        let conn = test_conn();
        let a = once(&conn, "n", t(2026, 7, 18, 6, 0), t(2026, 7, 17, 0, 0));
        let id = a.id.to_string();
        let claimed = scheduler_step(&conn, t(2026, 7, 18, 6, 0));
        assert_eq!(claimed.len(), 1);

        // Edit while running: allowed, applies from the next occurrence — the
        // in-flight run keeps its captured prompt/context.
        let patch = AutomationPatch {
            prompt: Some("nový prompt".into()),
            ..Default::default()
        };
        assert!(repo_update(&conn, &id, &patch, t(2026, 7, 18, 6, 5)).is_ok());

        // Disable while running: the run finishes, but nothing new is claimed.
        crate::automations_api::repo_set_enabled(&conn, &id, false, t(2026, 7, 18, 6, 5)).unwrap();
        assert!(scheduler_step(&conn, t(2026, 7, 19, 6, 0)).is_empty());

        // Delete while running: deterministic 409 until the run finishes.
        assert!(matches!(repo_delete(&conn, &id), Err(RepoError::ActiveRun)));
        crate::automations_api::repo_finish_run(
            &conn,
            &claimed[0].1.id.to_string(),
            &id,
            AutomationRunStatus::Completed,
            Some("ok"),
            t(2026, 7, 18, 6, 10),
        )
        .unwrap();
        assert!(repo_delete(&conn, &id).is_ok());
    }

    #[test]
    fn audit_and_run_payloads_never_contain_the_prompt() {
        let conn = test_conn();
        let secret_prompt = "SECRET-PROMPT-najdi vsetky faktury od ACME";
        let a = repo_create(
            &conn,
            "Faktúry",
            secret_prompt,
            "Europe/Bratislava",
            &AutomationSchedule::Once {
                at: t(2026, 7, 18, 6, 0),
            },
            true,
            t(2026, 7, 17, 0, 0),
        )
        .unwrap();
        // Catch-up claim (3 h late) so lifecycle audit rows exist to inspect.
        let claimed = scheduler_step(&conn, t(2026, 7, 18, 9, 0));
        assert_eq!(claimed.len(), 1);
        crate::automations_api::repo_finish_run(
            &conn,
            &claimed[0].1.id.to_string(),
            &a.id.to_string(),
            AutomationRunStatus::Failed,
            Some("Model error: connection refused"),
            t(2026, 7, 18, 9, 1),
        )
        .unwrap();

        // No audit payload may carry the stored prompt.
        let mut stmt = conn.prepare("SELECT payload FROM audit_entries").unwrap();
        let payloads: Vec<String> = stmt
            .query_map([], |r| r.get::<_, Option<String>>(0))
            .unwrap()
            .flatten()
            .flatten()
            .collect();
        assert!(!payloads.is_empty());
        for p in &payloads {
            assert!(!p.contains(secret_prompt), "prompt leaked into audit: {p}");
        }
        // Run rows keep only the concise redacted summary.
        let runs = repo_recent_runs(&conn, &a.id.to_string(), 10).unwrap();
        assert_eq!(
            runs[0].result_summary.as_deref(),
            Some("Model error: connection refused")
        );
    }

    #[test]
    fn next_due_ordering_and_clock_jump_forward() {
        let conn = test_conn();
        once(&conn, "b", t(2026, 7, 18, 8, 0), t(2026, 7, 17, 0, 0));
        once(&conn, "a", t(2026, 7, 18, 6, 0), t(2026, 7, 17, 0, 0));
        assert_eq!(next_due_at(&conn), Some(t(2026, 7, 18, 6, 0)));
        // Clock jumps past both → both fire in one pass (each within 24 h).
        let claimed = scheduler_step(&conn, t(2026, 7, 18, 9, 0));
        assert_eq!(claimed.len(), 2);
        assert_eq!(next_due_at(&conn), None);
    }
}
