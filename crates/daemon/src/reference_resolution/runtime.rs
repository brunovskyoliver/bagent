use super::{
    parse_resolver_mode_with_status, ConversationalReferenceResolver, EvidenceOrchestratorFlag,
    ResolverMode, ResolverModeParseStatus,
};
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

struct MaintenanceState {
    last_opportunistic: Mutex<Option<Instant>>,
    failed: AtomicBool,
    started: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct ResolverRuntime {
    mode: ResolverMode,
    resolver: Arc<dyn ConversationalReferenceResolver>,
    maintenance: Arc<MaintenanceState>,
}

impl ResolverRuntime {
    fn new(mode: ResolverMode, resolver: Arc<dyn ConversationalReferenceResolver>) -> Self {
        Self {
            mode,
            resolver,
            maintenance: Arc::new(MaintenanceState {
                last_opportunistic: Mutex::new(None),
                failed: AtomicBool::new(false),
                started: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) const fn mode(&self) -> ResolverMode {
        self.mode
    }

    pub(crate) async fn startup(&self) -> Result<(), super::ResolverFault> {
        if self.maintenance.failed.load(Ordering::Acquire) {
            return Err(super::ResolverFault::Unavailable);
        }
        self.resolver.startup().await
    }

    pub(crate) async fn opportunistic_prune(&self) -> Result<(), super::ResolverFault> {
        if self.maintenance.failed.load(Ordering::Acquire) {
            return Err(super::ResolverFault::Unavailable);
        }
        let mut last = self.maintenance.last_opportunistic.lock().await;
        if last.is_some_and(|instant| instant.elapsed() < Duration::from_secs(60)) {
            return Ok(());
        }
        *last = Some(Instant::now());
        drop(last);
        match self.resolver.maintenance_prune(now_ms()).await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.maintenance.failed.store(true, Ordering::Release);
                Err(error)
            }
        }
    }

    pub(crate) fn start_scheduled_prune(&self) {
        if self.maintenance.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let runtime = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            interval.tick().await;
            loop {
                interval.tick().await;
                if runtime.opportunistic_prune().await.is_err() {
                    break;
                }
            }
        });
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

impl fmt::Debug for ResolverRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolverRuntime")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(crate) enum RuntimeSelection {
    LegacyStage9,
    ResolverOff {
        parse_status: ResolverModeParseStatus,
    },
    Enabled {
        runtime: ResolverRuntime,
        parse_status: ResolverModeParseStatus,
    },
    Unavailable {
        mode: ResolverMode,
        parse_status: ResolverModeParseStatus,
    },
}

impl fmt::Debug for RuntimeSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LegacyStage9 => formatter.write_str("LegacyStage9"),
            Self::ResolverOff { parse_status } => formatter
                .debug_struct("ResolverOff")
                .field("parse_status", parse_status)
                .finish(),
            Self::Enabled {
                runtime,
                parse_status,
            } => formatter
                .debug_struct("Enabled")
                .field("runtime", runtime)
                .field("parse_status", parse_status)
                .finish(),
            Self::Unavailable { mode, parse_status } => formatter
                .debug_struct("Unavailable")
                .field("mode", mode)
                .field("parse_status", parse_status)
                .finish(),
        }
    }
}

impl RuntimeSelection {
    pub(crate) fn into_runtime(self) -> Option<ResolverRuntime> {
        match self {
            Self::Enabled { runtime, .. } => Some(runtime),
            Self::LegacyStage9 | Self::ResolverOff { .. } | Self::Unavailable { .. } => None,
        }
    }

    pub(crate) const fn mode_label(&self) -> &'static str {
        match self {
            Self::LegacyStage9 => ResolverMode::LegacyStage9.as_str(),
            Self::ResolverOff { .. } => ResolverMode::Off.as_str(),
            Self::Enabled { runtime, .. } => runtime.mode().as_str(),
            Self::Unavailable { mode, .. } => mode.as_str(),
        }
    }

    pub(crate) const fn selection_label(&self) -> &'static str {
        match self {
            Self::LegacyStage9 => "legacy_stage9",
            Self::ResolverOff { .. } => "resolver_off",
            Self::Enabled { .. } => "resolver_enabled",
            Self::Unavailable { .. } => "resolver_unavailable",
        }
    }

    pub(crate) const fn parse_status_label(&self) -> &'static str {
        match self {
            Self::LegacyStage9 => "bypassed",
            Self::ResolverOff { parse_status }
            | Self::Enabled { parse_status, .. }
            | Self::Unavailable { parse_status, .. } => parse_status.as_str(),
        }
    }
}

/// Select the resolver startup state. The top-level rollback flag is checked
/// before either lazy supplier is evaluated.
pub(crate) fn select_runtime<Subordinate, Factory, Error>(
    flag: EvidenceOrchestratorFlag,
    subordinate_supplier: Subordinate,
    resolver_factory: Factory,
) -> RuntimeSelection
where
    Subordinate: FnOnce() -> Option<String>,
    Factory: FnOnce(ResolverMode) -> Result<Arc<dyn ConversationalReferenceResolver>, Error>,
{
    if flag == EvidenceOrchestratorFlag::Disabled {
        return RuntimeSelection::LegacyStage9;
    }

    let subordinate = subordinate_supplier();
    let parsed = parse_resolver_mode_with_status(subordinate.as_deref());
    if parsed.mode() == ResolverMode::Off {
        return RuntimeSelection::ResolverOff {
            parse_status: parsed.status(),
        };
    }

    match resolver_factory(parsed.mode()) {
        Ok(resolver) => RuntimeSelection::Enabled {
            runtime: ResolverRuntime::new(parsed.mode(), resolver),
            parse_status: parsed.status(),
        },
        Err(_) => RuntimeSelection::Unavailable {
            mode: parsed.mode(),
            parse_status: parsed.status(),
        },
    }
}
