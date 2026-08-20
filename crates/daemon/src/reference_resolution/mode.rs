#![allow(dead_code)]

pub(crate) use crate::agent_exec::EvidenceOrchestratorFlag;

pub(crate) const REFERENCE_RESOLVER_MODE_ENV: &str = "BAGENT_REFERENCE_RESOLVER_MODE";
pub(crate) const DEFAULT_RESOLVER_MODE: ResolverMode = ResolverMode::Off;

/// Closed resolver startup selection. `LegacyStage9` is the result of the
/// higher-precedence Stage 9 rollback flag and is not a subordinate grammar
/// value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolverMode {
    LegacyStage9,
    Off,
    Persistence,
    Observe,
    Enforce,
    #[cfg(feature = "stage8-acceptance")]
    FixtureEnforcement,
}

impl ResolverMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyStage9 => "legacy_stage9",
            Self::Off => "off",
            Self::Persistence => "persistence",
            Self::Observe => "observe",
            Self::Enforce => "enforce",
            #[cfg(feature = "stage8-acceptance")]
            Self::FixtureEnforcement => "fixture_enforcement",
        }
    }
}

/// Parse the exact production subordinate grammar. Missing, malformed,
/// whitespace-padded, case-variant, and acceptance-only values fail closed to
/// the contracts-only default.
pub(crate) fn parse_resolver_mode(value: Option<&str>) -> ResolverMode {
    parse_resolver_mode_with_status(value).mode()
}

/// Content-free startup classification for the subordinate resolver setting.
/// The invalid input itself is intentionally not retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolverModeParseStatus {
    Absent,
    Valid,
    Invalid,
}

impl ResolverModeParseStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Valid => "valid",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParsedResolverMode {
    mode: ResolverMode,
    status: ResolverModeParseStatus,
}

impl ParsedResolverMode {
    pub(crate) const fn mode(self) -> ResolverMode {
        self.mode
    }

    pub(crate) const fn status(self) -> ResolverModeParseStatus {
        self.status
    }
}

/// Parse the subordinate value and return only its closed mode plus a
/// content-free status for startup reporting.
pub(crate) fn parse_resolver_mode_with_status(value: Option<&str>) -> ParsedResolverMode {
    let (mode, status) = match value {
        None => (DEFAULT_RESOLVER_MODE, ResolverModeParseStatus::Absent),
        Some("off") => (ResolverMode::Off, ResolverModeParseStatus::Valid),
        Some("persistence") => (ResolverMode::Persistence, ResolverModeParseStatus::Valid),
        Some("observe") => (ResolverMode::Observe, ResolverModeParseStatus::Valid),
        Some("enforce") => (ResolverMode::Enforce, ResolverModeParseStatus::Valid),
        Some(_) => (DEFAULT_RESOLVER_MODE, ResolverModeParseStatus::Invalid),
    };
    ParsedResolverMode { mode, status }
}

/// Apply the Stage 9 flag before considering the subordinate resolver mode.
/// The function is deliberately pure so flag precedence can be tested without
/// reading process configuration.
pub(crate) fn select_resolver_mode(
    flag: EvidenceOrchestratorFlag,
    subordinate: Option<&str>,
) -> ResolverMode {
    match flag {
        EvidenceOrchestratorFlag::Disabled => ResolverMode::LegacyStage9,
        EvidenceOrchestratorFlag::Enabled => parse_resolver_mode(subordinate),
    }
}
