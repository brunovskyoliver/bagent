//! Durable, one-use confirmation state and the closed continuation values
//! shared by the resolver and its private repository seam.

use std::fmt;
use unicode_normalization::UnicodeNormalization;

pub(crate) const NORMALIZATION_VERSION: u32 = 1;
pub(crate) const COMPATIBILITY_EPOCH: u32 = 1;
pub(crate) const CONFIRMATION_TTL_MS: i64 = 300_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfirmationRequestKind {
    Confirm,
    Edit,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ConfirmationDisposition {
    Unchanged,
    Pending {
        confirmation_id: String,
        proposal: String,
        expires_at_ms: i64,
    },
    Confirmed {
        referent_id: String,
        mention_id: Option<String>,
        provider_scope: String,
    },
    EditAccepted {
        replacement: Vec<u8>,
    },
    BlockedAlreadyConsumed,
    BlockedTermMismatch,
    BlockedExpired,
    BlockedInvalidationFailure,
    BlockedBindingFailure,
    BlockedInteractiveAction,
    Unavailable,
}

impl fmt::Debug for ConfirmationDisposition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unchanged => formatter.write_str("Unchanged"),
            Self::Pending {
                confirmation_id,
                expires_at_ms,
                ..
            } => formatter
                .debug_struct("Pending")
                .field("confirmation_id", confirmation_id)
                .field("has_proposal", &true)
                .field("expires_at_ms", expires_at_ms)
                .finish(),
            Self::Confirmed {
                referent_id,
                mention_id,
                provider_scope,
            } => formatter
                .debug_struct("Confirmed")
                .field("referent_id", &referent_id)
                .field("mention_id", &mention_id)
                .field("provider_scope", &provider_scope)
                .finish(),
            Self::EditAccepted { replacement } => formatter
                .debug_struct("EditAccepted")
                .field("replacement_bytes", &replacement.len())
                .finish(),
            Self::BlockedAlreadyConsumed => formatter.write_str("BlockedAlreadyConsumed"),
            Self::BlockedTermMismatch => formatter.write_str("BlockedTermMismatch"),
            Self::BlockedExpired => formatter.write_str("BlockedExpired"),
            Self::BlockedInvalidationFailure => formatter.write_str("BlockedInvalidationFailure"),
            Self::BlockedBindingFailure => formatter.write_str("BlockedBindingFailure"),
            Self::BlockedInteractiveAction => formatter.write_str("BlockedInteractiveAction"),
            Self::Unavailable => formatter.write_str("Unavailable"),
        }
    }
}

impl ConfirmationDisposition {
    pub(crate) const fn is_blocked(&self) -> bool {
        matches!(
            self,
            Self::BlockedAlreadyConsumed
                | Self::BlockedTermMismatch
                | Self::BlockedExpired
                | Self::BlockedInvalidationFailure
                | Self::BlockedBindingFailure
                | Self::BlockedInteractiveAction
                | Self::Unavailable
        )
    }

    pub(crate) const fn structural_outcome(&self) -> Option<&'static str> {
        Some(match self {
            Self::BlockedAlreadyConsumed => "already_consumed",
            Self::BlockedTermMismatch => "term_mismatch",
            Self::BlockedExpired => "expired",
            Self::BlockedInvalidationFailure => "invalidation_failed",
            Self::BlockedBindingFailure => "binding_failed",
            Self::BlockedInteractiveAction => "interactive_action_blocked",
            Self::Unavailable => "unavailable",
            Self::Unchanged
            | Self::Pending { .. }
            | Self::Confirmed { .. }
            | Self::EditAccepted { .. } => return None,
        })
    }
}

/// NFC, locale-independent lowercase, collapsed Unicode whitespace, and
/// bounded edge punctuation are the only normalization rules in V1.
pub(crate) fn normalize_term(value: &str) -> Vec<u8> {
    value
        .nfc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|ch: char| " \t\r\n.,;:!?()[]{}<>\"'`".contains(ch))
        .as_bytes()
        .to_vec()
}

pub(crate) fn is_safe_public_proposal(value: &str) -> bool {
    let lower = value.to_lowercase();
    if [
        "account",
        "credential",
        "password",
        "secret",
        "serial",
        "tracking",
        "order",
        "token",
    ]
    .iter()
    .any(|forbidden| lower.contains(forbidden))
    {
        return false;
    }
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return true;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let private_ip = host.parse::<std::net::IpAddr>().ok().is_some_and(|ip| {
        ip.is_loopback()
            || match ip {
                std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
                std::net::IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local(),
            }
    });
    url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && !private_ip
        && !matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// Structural event labels are deliberately content-free.  The exact public
/// proposal is carried only by the pending event assembled by its caller.
pub(crate) fn terminal_event_sequence(disposition: &ConfirmationDisposition) -> Vec<&'static str> {
    match disposition {
        ConfirmationDisposition::EditAccepted { .. } => {
            vec!["reference_resolution(edit_accepted,proceeding)"]
        }
        ConfirmationDisposition::BlockedTermMismatch => {
            vec!["reference_resolution(term_mismatch,blocked)", "done"]
        }
        ConfirmationDisposition::BlockedAlreadyConsumed => {
            vec!["reference_resolution(already_consumed,blocked)", "done"]
        }
        ConfirmationDisposition::BlockedExpired => {
            vec!["reference_resolution(expired,blocked)", "done"]
        }
        ConfirmationDisposition::BlockedInvalidationFailure => {
            vec!["reference_resolution(invalidation_failed,blocked)", "done"]
        }
        ConfirmationDisposition::BlockedBindingFailure => {
            vec!["reference_resolution(binding_failed,blocked)", "done"]
        }
        ConfirmationDisposition::BlockedInteractiveAction => {
            vec![
                "reference_resolution(interactive_action_blocked,blocked)",
                "done",
            ]
        }
        ConfirmationDisposition::Unavailable => {
            vec!["reference_resolution(unavailable,blocked)", "done"]
        }
        ConfirmationDisposition::Confirmed { .. } => {
            vec!["reference_resolution(confirm_accepted,proceeding)"]
        }
        ConfirmationDisposition::Pending { .. } => {
            vec!["reference_resolution(pending_confirmation,proceeding)"]
        }
        ConfirmationDisposition::Unchanged => Vec::new(),
    }
}
