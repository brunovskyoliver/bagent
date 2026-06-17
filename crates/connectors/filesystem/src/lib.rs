pub mod open;
pub mod path_policy;
pub mod search;
pub mod types;

pub use path_policy::PathPolicy;
pub use types::{
    FileKind, FileMetadataResponse, FileSearchRequest, FileSearchResponse, FileSearchResult,
    MatchType, OpenAppRequest, OpenFileRequest, OpenFileWithRequest, OpenResponse, ReadTextRequest,
    ReadTextResponse,
};

/// Thin connector wrapping the path policy.
/// Clone-safe: all fields are `Clone`.
#[derive(Clone)]
pub struct FsConnector {
    pub policy: PathPolicy,
}

impl FsConnector {
    /// Build with the default user-home policy. Returns `Err` only if home dir cannot be determined.
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            policy: PathPolicy::default_for_user_home()?,
        })
    }

    /// Returns true as long as we could build a policy (home dir exists).
    pub fn is_accessible(&self) -> bool {
        !self.policy.allowed_roots.is_empty()
    }
}
