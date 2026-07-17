//! bagent-automations: typed schedule semantics, domain model, and locked
//! operational policies for local scheduled automations. Pure logic — the
//! daemon owns persistence, the scheduler loop, HTTP, and SSE.

pub mod model;
pub mod policy;
pub mod schedule;

pub use model::{
    Automation, AutomationExecutionContext, AutomationId, AutomationRun, AutomationRunId,
    AutomationRunStatus,
};
pub use schedule::{
    missed_run_decision, parse_timezone, resolve_local, AutomationSchedule, MissedRunDecision,
    RecurrenceRule, ScheduleError, Weekday, CATCH_UP_WINDOW,
};
