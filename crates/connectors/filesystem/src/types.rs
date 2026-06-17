use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchRequest {
    /// Primary query string (kept for backwards compat with existing callers).
    /// Converted to `terms` internally; if both `query` and `terms` are set,
    /// `terms` wins.
    pub query: String,
    /// Multiple search terms — OR semantics, each run as a separate Spotlight pass.
    /// When non-empty the Spotlight backend is preferred; walkdir is the fallback.
    #[serde(default)]
    pub terms: Vec<String>,
    pub roots: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub search_names: bool,
    #[serde(default)]
    pub search_contents: bool,
    pub extensions: Option<Vec<String>>,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default = "default_20")]
    pub max_results: usize,
    pub max_depth: Option<usize>,
}

impl Default for FileSearchRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            terms: vec![],
            roots: None,
            search_names: true,
            search_contents: false,
            extensions: None,
            include_hidden: false,
            max_results: 20,
            max_depth: None,
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_20() -> usize {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchResponse {
    pub query: String,
    pub results: Vec<FileSearchResult>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchResult {
    pub path: String,
    pub display_name: String,
    pub parent: Option<String>,
    pub kind: FileKind,
    pub mime: Option<String>,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub match_type: MatchType,
    pub matched_line: Option<String>,
    pub line_number: Option<u64>,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    File,
    Directory,
    Package,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    FileName,
    Content,
    Path,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadTextRequest {
    pub path: String,
    #[serde(default)]
    pub max_bytes: Option<usize>,
    #[serde(default)]
    pub around_line: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadTextResponse {
    pub path: String,
    pub mime: Option<String>,
    pub truncated: bool,
    pub content: String,
    /// Always true — all local file contents are treated as private.
    pub pii: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadataResponse {
    pub path: String,
    pub display_name: String,
    pub parent: Option<String>,
    pub kind: FileKind,
    pub mime: Option<String>,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenFileRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenFileWithRequest {
    pub path: String,
    pub app: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAppRequest {
    pub app: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenResponse {
    pub ok: bool,
    pub path: Option<String>,
    pub app: Option<String>,
    pub action: String,
}
