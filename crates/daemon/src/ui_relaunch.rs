use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const TRANSFER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TransferPhase {
    Reserved,
    Ready,
    OldFenced,
    Active,
    Acknowledged,
    RolledBack,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStatus {
    Unknown,
    Reserved,
    Ready,
    OldFenced,
    Active,
    Acknowledged,
    RolledBack,
    Expired,
}

impl TransferStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Reserved => "reserved",
            Self::Ready => "ready",
            Self::OldFenced => "old_fenced",
            Self::Active => "active",
            Self::Acknowledged => "acknowledged",
            Self::RolledBack => "rolled_back",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiConsumerTransferError {
    StaleConsumer,
    DuplicateReplacement,
    InvalidTransfer,
    NotReady,
    AlreadyAcknowledged,
    Expired,
}

#[derive(Debug, Clone)]
struct PendingTransfer {
    identity: String,
    old_fence: String,
    replacement_fence: String,
    deadline: Instant,
    phase: TransferPhase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedPendingTransfer {
    identity: String,
    old_fence: String,
    replacement_fence: String,
    deadline_unix_ms: u64,
    phase: TransferPhase,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PersistedUiConsumerAuthority {
    active_fence: Option<String>,
    pending: Option<PersistedPendingTransfer>,
}

#[derive(Debug, Default)]
pub struct UiConsumerAuthority {
    active_fence: Option<String>,
    pending: Option<PendingTransfer>,
}

impl UiConsumerAuthority {
    pub fn load(path: &Path, now: Instant) -> std::io::Result<Self> {
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error),
        };
        let saved = serde_json::from_slice::<PersistedUiConsumerAuthority>(&data)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let now_unix_ms = unix_millis(SystemTime::now());
        Ok(Self {
            active_fence: saved.active_fence,
            pending: saved.pending.map(|pending| PendingTransfer {
                identity: pending.identity,
                old_fence: pending.old_fence,
                replacement_fence: pending.replacement_fence,
                deadline: if pending.deadline_unix_ms > now_unix_ms {
                    now + Duration::from_millis(pending.deadline_unix_ms - now_unix_ms)
                } else {
                    now
                },
                phase: pending.phase,
            }),
        })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let saved = PersistedUiConsumerAuthority {
            active_fence: self.active_fence.clone(),
            pending: self
                .pending
                .as_ref()
                .map(|pending| PersistedPendingTransfer {
                    identity: pending.identity.clone(),
                    old_fence: pending.old_fence.clone(),
                    replacement_fence: pending.replacement_fence.clone(),
                    deadline_unix_ms: unix_millis(SystemTime::now()).saturating_add(
                        pending
                            .deadline
                            .saturating_duration_since(Instant::now())
                            .as_millis() as u64,
                    ),
                    phase: pending.phase,
                }),
        };
        let data = serde_json::to_vec(&saved).map_err(std::io::Error::other)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, data)?;
        fs::rename(temporary, path)
    }

    pub fn claim_snapshot(
        &mut self,
        fence: &str,
        now: Instant,
    ) -> Result<(), UiConsumerTransferError> {
        self.expire(now);
        if let Some(pending) = &self.pending {
            if matches!(
                pending.phase,
                TransferPhase::Expired | TransferPhase::RolledBack
            ) {
                return if self.active_fence.as_deref() == Some(fence) {
                    Ok(())
                } else {
                    Err(UiConsumerTransferError::StaleConsumer)
                };
            }
            if fence == pending.old_fence
                && matches!(
                    pending.phase,
                    TransferPhase::Reserved | TransferPhase::Ready
                )
            {
                return Ok(());
            }
            if fence == pending.replacement_fence
                && matches!(
                    pending.phase,
                    TransferPhase::Active | TransferPhase::Acknowledged
                )
            {
                return Ok(());
            }
            return Err(UiConsumerTransferError::StaleConsumer);
        }
        match self.active_fence.as_deref() {
            None => self.active_fence = Some(fence.to_owned()),
            Some(active) if active == fence => {}
            Some(_) => return Err(UiConsumerTransferError::StaleConsumer),
        }
        Ok(())
    }

    pub fn reserve(
        &mut self,
        identity: &str,
        old_fence: &str,
        replacement_fence: &str,
        now: Instant,
    ) -> Result<(), UiConsumerTransferError> {
        self.expire(now);
        if let Some(pending) = &self.pending {
            if pending.identity == identity
                || !matches!(
                    pending.phase,
                    TransferPhase::Expired
                        | TransferPhase::RolledBack
                        | TransferPhase::Acknowledged
                )
            {
                return Err(UiConsumerTransferError::DuplicateReplacement);
            }
        }
        if self.active_fence.as_deref() != Some(old_fence) {
            return Err(UiConsumerTransferError::StaleConsumer);
        }
        if identity.is_empty() || replacement_fence.is_empty() || old_fence == replacement_fence {
            return Err(UiConsumerTransferError::InvalidTransfer);
        }
        self.pending = Some(PendingTransfer {
            identity: identity.to_owned(),
            old_fence: old_fence.to_owned(),
            replacement_fence: replacement_fence.to_owned(),
            deadline: now + TRANSFER_TIMEOUT,
            phase: TransferPhase::Reserved,
        });
        Ok(())
    }

    pub fn status(&mut self, identity: &str, now: Instant) -> TransferStatus {
        self.expire(now);
        self.pending
            .as_ref()
            .filter(|pending| pending.identity == identity)
            .map(|pending| match pending.phase {
                TransferPhase::Reserved => TransferStatus::Reserved,
                TransferPhase::Ready => TransferStatus::Ready,
                TransferPhase::OldFenced => TransferStatus::OldFenced,
                TransferPhase::Active => TransferStatus::Active,
                TransferPhase::Acknowledged => TransferStatus::Acknowledged,
                TransferPhase::RolledBack => TransferStatus::RolledBack,
                TransferPhase::Expired => TransferStatus::Expired,
            })
            .unwrap_or(TransferStatus::Unknown)
    }

    pub fn ready(
        &mut self,
        identity: &str,
        replacement_fence: &str,
        now: Instant,
    ) -> Result<(), UiConsumerTransferError> {
        self.require(identity, replacement_fence, now, TransferPhase::Reserved)?
            .phase = TransferPhase::Ready;
        Ok(())
    }

    pub fn refetch_reserved(
        &mut self,
        identity: &str,
        replacement_fence: &str,
        now: Instant,
    ) -> Result<(), UiConsumerTransferError> {
        let pending = self
            .pending
            .as_ref()
            .filter(|pending| pending.identity == identity)
            .ok_or(UiConsumerTransferError::InvalidTransfer)?;
        if pending.deadline <= now || pending.phase == TransferPhase::Expired {
            return Err(UiConsumerTransferError::Expired);
        }
        if pending.replacement_fence != replacement_fence {
            return Err(UiConsumerTransferError::StaleConsumer);
        }
        if !matches!(
            pending.phase,
            TransferPhase::Reserved | TransferPhase::Ready
        ) {
            return Err(UiConsumerTransferError::NotReady);
        }
        Ok(())
    }

    pub fn activate(
        &mut self,
        identity: &str,
        replacement_fence: &str,
        now: Instant,
    ) -> Result<(), UiConsumerTransferError> {
        let replacement_fence = {
            let pending =
                self.require(identity, replacement_fence, now, TransferPhase::OldFenced)?;
            pending.phase = TransferPhase::Active;
            pending.replacement_fence.clone()
        };
        self.active_fence = Some(replacement_fence);
        Ok(())
    }

    pub fn fence_old(
        &mut self,
        identity: &str,
        old_fence: &str,
        now: Instant,
    ) -> Result<(), UiConsumerTransferError> {
        self.expire(now);
        let pending = self
            .pending
            .as_mut()
            .filter(|pending| pending.identity == identity)
            .ok_or(UiConsumerTransferError::InvalidTransfer)?;
        if pending.deadline <= now || pending.phase == TransferPhase::Expired {
            return Err(UiConsumerTransferError::Expired);
        }
        if pending.old_fence != old_fence {
            return Err(UiConsumerTransferError::StaleConsumer);
        }
        match pending.phase {
            TransferPhase::Ready => pending.phase = TransferPhase::OldFenced,
            TransferPhase::OldFenced => return Ok(()),
            _ => return Err(UiConsumerTransferError::NotReady),
        }
        Ok(())
    }

    pub fn acknowledge(
        &mut self,
        identity: &str,
        replacement_fence: &str,
        now: Instant,
    ) -> Result<(), UiConsumerTransferError> {
        let pending = self.require(identity, replacement_fence, now, TransferPhase::Active)?;
        pending.phase = TransferPhase::Acknowledged;
        Ok(())
    }

    pub fn rollback(
        &mut self,
        identity: &str,
        old_fence: &str,
        now: Instant,
    ) -> Result<(), UiConsumerTransferError> {
        self.expire(now);
        let pending = self
            .pending
            .as_mut()
            .filter(|pending| pending.identity == identity)
            .ok_or(UiConsumerTransferError::InvalidTransfer)?;
        if pending.deadline <= now || pending.phase == TransferPhase::Expired {
            return Err(UiConsumerTransferError::Expired);
        }
        if pending.old_fence != old_fence {
            return Err(UiConsumerTransferError::StaleConsumer);
        }
        if !matches!(
            pending.phase,
            TransferPhase::Reserved | TransferPhase::Ready | TransferPhase::Active
        ) {
            return Err(if pending.phase == TransferPhase::Acknowledged {
                UiConsumerTransferError::AlreadyAcknowledged
            } else {
                UiConsumerTransferError::NotReady
            });
        }
        let old_fence = pending.old_fence.clone();
        pending.phase = TransferPhase::RolledBack;
        self.active_fence = Some(old_fence);
        Ok(())
    }

    pub fn active_fence(&mut self, now: Instant) -> Option<String> {
        self.expire(now);
        self.active_fence.clone()
    }

    pub fn replacement_fence(&mut self, identity: &str, now: Instant) -> Option<String> {
        self.expire(now);
        self.pending
            .as_ref()
            .filter(|pending| pending.identity == identity)
            .map(|pending| pending.replacement_fence.clone())
    }

    pub fn transfer_exists(&mut self, identity: &str, now: Instant) -> bool {
        self.status(identity, now) != TransferStatus::Unknown
    }

    fn require(
        &mut self,
        identity: &str,
        fence: &str,
        now: Instant,
        expected: TransferPhase,
    ) -> Result<&mut PendingTransfer, UiConsumerTransferError> {
        self.expire(now);
        let pending = self
            .pending
            .as_mut()
            .filter(|pending| pending.identity == identity)
            .ok_or(UiConsumerTransferError::InvalidTransfer)?;
        if pending.deadline <= now || pending.phase == TransferPhase::Expired {
            return Err(UiConsumerTransferError::Expired);
        }
        let fence_matches = match expected {
            TransferPhase::Reserved
            | TransferPhase::Ready
            | TransferPhase::OldFenced
            | TransferPhase::Active => fence == pending.replacement_fence,
            TransferPhase::RolledBack => fence == pending.old_fence,
            TransferPhase::Acknowledged | TransferPhase::Expired => true,
        };
        if !fence_matches {
            return Err(UiConsumerTransferError::StaleConsumer);
        }
        if pending.phase != expected {
            return Err(match pending.phase {
                TransferPhase::Acknowledged => UiConsumerTransferError::AlreadyAcknowledged,
                TransferPhase::Expired => UiConsumerTransferError::Expired,
                _ => UiConsumerTransferError::NotReady,
            });
        }
        Ok(pending)
    }

    fn expire(&mut self, now: Instant) {
        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        if pending.deadline > now
            || matches!(
                pending.phase,
                TransferPhase::Acknowledged | TransferPhase::RolledBack | TransferPhase::Expired
            )
        {
            return;
        }
        if self.active_fence.as_deref() == Some(pending.replacement_fence.as_str()) {
            self.active_fence = Some(pending.old_fence.clone());
        }
        pending.phase = TransferPhase::Expired;
    }
}

fn unix_millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn new_transfer_identity() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_does_not_fence_old_until_activation() {
        let start = Instant::now();
        let mut authority = UiConsumerAuthority::default();
        authority.claim_snapshot("old", start).unwrap();
        authority.reserve("handoff", "old", "new", start).unwrap();
        assert_eq!(authority.active_fence(start), Some("old".into()));
        authority.ready("handoff", "new", start).unwrap();
        authority.fence_old("handoff", "old", start).unwrap();
        authority.activate("handoff", "new", start).unwrap();
        assert_eq!(authority.active_fence(start), Some("new".into()));
        assert_eq!(authority.status("handoff", start), TransferStatus::Active);
    }

    #[test]
    fn duplicate_and_stale_replacements_are_rejected() {
        let start = Instant::now();
        let mut authority = UiConsumerAuthority::default();
        authority.claim_snapshot("old", start).unwrap();
        assert_eq!(
            authority.claim_snapshot("stale", start),
            Err(UiConsumerTransferError::StaleConsumer)
        );
        authority.reserve("handoff", "old", "new", start).unwrap();
        assert_eq!(
            authority.reserve("other", "old", "duplicate", start),
            Err(UiConsumerTransferError::DuplicateReplacement)
        );
        assert_eq!(
            authority.reserve("stale", "wrong", "newer", start),
            Err(UiConsumerTransferError::DuplicateReplacement)
        );
    }

    #[test]
    fn timeout_restores_old_and_rejects_late_activation() {
        let start = Instant::now();
        let mut authority = UiConsumerAuthority::default();
        authority.claim_snapshot("old", start).unwrap();
        authority.reserve("handoff", "old", "new", start).unwrap();
        authority.ready("handoff", "new", start).unwrap();
        authority.fence_old("handoff", "old", start).unwrap();
        assert_eq!(
            authority.status("handoff", start + TRANSFER_TIMEOUT),
            TransferStatus::Expired
        );
        assert_eq!(
            authority.active_fence(start + TRANSFER_TIMEOUT),
            Some("old".into())
        );
        assert_eq!(
            authority.activate("handoff", "new", start + TRANSFER_TIMEOUT),
            Err(UiConsumerTransferError::Expired)
        );
    }

    #[test]
    fn acknowledgement_is_single_use() {
        let start = Instant::now();
        let mut authority = UiConsumerAuthority::default();
        authority.claim_snapshot("old", start).unwrap();
        authority.reserve("handoff", "old", "new", start).unwrap();
        authority.ready("handoff", "new", start).unwrap();
        authority.fence_old("handoff", "old", start).unwrap();
        authority.activate("handoff", "new", start).unwrap();
        authority.acknowledge("handoff", "new", start).unwrap();
        assert_eq!(
            authority.acknowledge("handoff", "new", start),
            Err(UiConsumerTransferError::AlreadyAcknowledged)
        );
    }

    #[test]
    fn rollback_restores_old_after_activation_without_accepting_late_ack() {
        let start = Instant::now();
        let mut authority = UiConsumerAuthority::default();
        authority.claim_snapshot("old", start).unwrap();
        authority.reserve("handoff", "old", "new", start).unwrap();
        authority.ready("handoff", "new", start).unwrap();
        authority.fence_old("handoff", "old", start).unwrap();
        authority.activate("handoff", "new", start).unwrap();
        authority.rollback("handoff", "old", start).unwrap();
        assert_eq!(authority.active_fence(start), Some("old".into()));
        assert_eq!(
            authority.status("handoff", start),
            TransferStatus::RolledBack
        );
        assert_eq!(
            authority.acknowledge("handoff", "new", start),
            Err(UiConsumerTransferError::NotReady)
        );
    }

    #[test]
    fn acknowledged_replacement_remains_the_only_active_consumer() {
        let start = Instant::now();
        let mut authority = UiConsumerAuthority::default();
        authority.claim_snapshot("old", start).unwrap();
        authority.reserve("handoff", "old", "new", start).unwrap();
        authority.ready("handoff", "new", start).unwrap();
        authority.fence_old("handoff", "old", start).unwrap();
        authority.activate("handoff", "new", start).unwrap();
        authority.acknowledge("handoff", "new", start).unwrap();

        assert!(authority.claim_snapshot("new", start).is_ok());
        assert_eq!(
            authority.claim_snapshot("old", start),
            Err(UiConsumerTransferError::StaleConsumer)
        );
        assert_eq!(authority.active_fence(start), Some("new".into()));
    }

    #[test]
    fn acknowledged_consumer_can_start_a_later_transfer() {
        let start = Instant::now();
        let mut authority = UiConsumerAuthority::default();
        authority.claim_snapshot("old", start).unwrap();
        authority.reserve("first", "old", "new", start).unwrap();
        authority.ready("first", "new", start).unwrap();
        authority.fence_old("first", "old", start).unwrap();
        authority.activate("first", "new", start).unwrap();
        authority.acknowledge("first", "new", start).unwrap();
        assert!(authority.reserve("second", "new", "next", start).is_ok());
    }

    #[test]
    fn persisted_authority_restores_active_consumer_after_restart() {
        let start = Instant::now();
        let path =
            std::env::temp_dir().join(format!("bagent-ui-authority-{}.json", Uuid::new_v4()));
        let mut authority = UiConsumerAuthority::default();
        authority.claim_snapshot("old", start).unwrap();
        authority.save(&path).unwrap();

        let mut restored = UiConsumerAuthority::load(&path, start).unwrap();
        assert_eq!(restored.active_fence(start), Some("old".into()));
        assert_eq!(
            restored.claim_snapshot("stale", start),
            Err(UiConsumerTransferError::StaleConsumer)
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn malformed_persisted_authority_fails_closed() {
        let start = Instant::now();
        let path =
            std::env::temp_dir().join(format!("bagent-ui-authority-{}.json", Uuid::new_v4()));
        fs::write(&path, b"not-json").unwrap();

        assert!(UiConsumerAuthority::load(&path, start).is_err());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn terminal_transfers_reject_stale_claims_and_identity_replay() {
        let start = Instant::now();
        let mut authority = UiConsumerAuthority::default();
        authority.claim_snapshot("old", start).unwrap();
        authority.reserve("handoff", "old", "new", start).unwrap();
        authority.ready("handoff", "new", start).unwrap();
        assert_eq!(
            authority.status("handoff", start + TRANSFER_TIMEOUT),
            TransferStatus::Expired
        );
        assert!(authority
            .claim_snapshot("old", start + TRANSFER_TIMEOUT)
            .is_ok());
        assert_eq!(
            authority.claim_snapshot("stale", start + TRANSFER_TIMEOUT),
            Err(UiConsumerTransferError::StaleConsumer)
        );
        assert_eq!(
            authority.reserve("handoff", "old", "replayed", start + TRANSFER_TIMEOUT),
            Err(UiConsumerTransferError::DuplicateReplacement)
        );
        assert!(authority
            .reserve("later", "old", "later-fence", start + TRANSFER_TIMEOUT)
            .is_ok());
    }

    #[test]
    fn fencing_is_idempotent_when_old_and_replacement_race() {
        let start = Instant::now();
        let mut authority = UiConsumerAuthority::default();
        authority.claim_snapshot("old", start).unwrap();
        authority.reserve("handoff", "old", "new", start).unwrap();
        authority.ready("handoff", "new", start).unwrap();
        authority.fence_old("handoff", "old", start).unwrap();
        assert!(authority.fence_old("handoff", "old", start).is_ok());
        authority.activate("handoff", "new", start).unwrap();
        assert_eq!(
            authority.fence_old("handoff", "old", start),
            Err(UiConsumerTransferError::NotReady)
        );
    }
}
