use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedResource {
    Mail,
    Notes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedResourceFailure {
    Denied,
    Unavailable,
}

pub trait ProtectedResourceOpener: Send + Sync {
    fn open(&self, resource: ProtectedResource) -> Result<(), ProtectedResourceFailure>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DaemonFullDiskAccessOutcome {
    Granted,
    Denied,
    Indeterminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonFullDiskAccessSnapshot {
    pub mail: DaemonFullDiskAccessOutcome,
    pub notes: DaemonFullDiskAccessOutcome,
}

pub struct DaemonFullDiskAccessProbe {
    opener: Arc<dyn ProtectedResourceOpener>,
}

impl DaemonFullDiskAccessProbe {
    pub fn new(opener: Arc<dyn ProtectedResourceOpener>) -> Self {
        Self { opener }
    }

    pub fn production() -> Self {
        Self::new(Arc::new(FileProtectedResourceOpener::production()))
    }

    /// Deterministic acceptance-only adapter. This method is compiled only
    /// into the disposable acceptance binary; production has no fixture path.
    #[cfg(feature = "stage7a-acceptance")]
    pub fn acceptance_granted() -> Self {
        Self::new(Arc::new(AcceptanceGrantedResourceOpener))
    }

    pub fn probe(&self) -> DaemonFullDiskAccessSnapshot {
        DaemonFullDiskAccessSnapshot {
            mail: normalize(self.opener.open(ProtectedResource::Mail)),
            notes: normalize(self.opener.open(ProtectedResource::Notes)),
        }
    }
}

#[cfg(feature = "stage7a-acceptance")]
struct AcceptanceGrantedResourceOpener;

#[cfg(feature = "stage7a-acceptance")]
impl ProtectedResourceOpener for AcceptanceGrantedResourceOpener {
    fn open(&self, _resource: ProtectedResource) -> Result<(), ProtectedResourceFailure> {
        Ok(())
    }
}

fn normalize(result: Result<(), ProtectedResourceFailure>) -> DaemonFullDiskAccessOutcome {
    match result {
        Ok(()) => DaemonFullDiskAccessOutcome::Granted,
        Err(ProtectedResourceFailure::Denied) => DaemonFullDiskAccessOutcome::Denied,
        Err(ProtectedResourceFailure::Unavailable) => DaemonFullDiskAccessOutcome::Indeterminate,
    }
}

struct FileProtectedResourceOpener {
    mail_path: PathBuf,
    notes_path: PathBuf,
}

impl FileProtectedResourceOpener {
    fn production() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self {
            mail_path: home.join("Library/Mail/V10/MailData/Envelope Index"),
            notes_path: home
                .join("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite"),
        }
    }

    #[cfg(test)]
    fn with_paths(mail_path: impl Into<PathBuf>, notes_path: impl Into<PathBuf>) -> Self {
        Self {
            mail_path: mail_path.into(),
            notes_path: notes_path.into(),
        }
    }

    fn path(&self, resource: ProtectedResource) -> &Path {
        match resource {
            ProtectedResource::Mail => &self.mail_path,
            ProtectedResource::Notes => &self.notes_path,
        }
    }
}

impl ProtectedResourceOpener for FileProtectedResourceOpener {
    fn open(&self, resource: ProtectedResource) -> Result<(), ProtectedResourceFailure> {
        File::open(self.path(resource))
            .map(|_| ())
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::PermissionDenied => ProtectedResourceFailure::Denied,
                _ => ProtectedResourceFailure::Unavailable,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct StubOpener {
        mail: Mutex<Option<Result<(), ProtectedResourceFailure>>>,
        notes: Mutex<Option<Result<(), ProtectedResourceFailure>>>,
        calls: Mutex<Vec<ProtectedResource>>,
    }

    impl StubOpener {
        fn new(
            mail: Result<(), ProtectedResourceFailure>,
            notes: Result<(), ProtectedResourceFailure>,
        ) -> Self {
            Self {
                mail: Mutex::new(Some(mail)),
                notes: Mutex::new(Some(notes)),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl ProtectedResourceOpener for StubOpener {
        fn open(&self, resource: ProtectedResource) -> Result<(), ProtectedResourceFailure> {
            self.calls.lock().unwrap().push(resource);
            match resource {
                ProtectedResource::Mail => self.mail.lock().unwrap().take(),
                ProtectedResource::Notes => self.notes.lock().unwrap().take(),
            }
            .unwrap_or(Err(ProtectedResourceFailure::Unavailable))
        }
    }

    #[test]
    fn daemon_probe_returns_separate_normalized_resource_results() {
        let opener = Arc::new(StubOpener::new(
            Ok(()),
            Err(ProtectedResourceFailure::Denied),
        ));
        let probe = DaemonFullDiskAccessProbe::new(opener.clone());

        assert_eq!(
            probe.probe(),
            DaemonFullDiskAccessSnapshot {
                mail: DaemonFullDiskAccessOutcome::Granted,
                notes: DaemonFullDiskAccessOutcome::Denied,
            }
        );
        assert_eq!(
            *opener.calls.lock().unwrap(),
            vec![ProtectedResource::Mail, ProtectedResource::Notes]
        );
    }

    #[test]
    fn delayed_availability_and_failures_never_expose_raw_errors() {
        let probe = DaemonFullDiskAccessProbe::new(Arc::new(StubOpener::new(
            Err(ProtectedResourceFailure::Unavailable),
            Err(ProtectedResourceFailure::Unavailable),
        )));

        assert_eq!(
            probe.probe(),
            DaemonFullDiskAccessSnapshot {
                mail: DaemonFullDiskAccessOutcome::Indeterminate,
                notes: DaemonFullDiskAccessOutcome::Indeterminate,
            }
        );
    }

    #[test]
    fn production_opener_attempts_exact_resources_without_reading_content() {
        let temp = tempfile::tempdir().unwrap();
        let mail = temp.path().join("Envelope Index");
        let notes = temp.path().join("NoteStore.sqlite");
        std::fs::write(&mail, b"protected mail fixture").unwrap();
        std::fs::write(&notes, b"protected notes fixture").unwrap();

        let opener = FileProtectedResourceOpener::with_paths(&mail, &notes);
        assert_eq!(opener.open(ProtectedResource::Mail), Ok(()));
        assert_eq!(opener.open(ProtectedResource::Notes), Ok(()));
    }
}
