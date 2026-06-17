use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

const DANGEROUS_EXTENSIONS: &[&str] = &[
    "app", "command", "tool", "sh", "zsh", "bash", "py", "js", "scpt", "workflow", "pkg", "dmg",
];

/// Policy controlling which paths the filesystem connector may access.
#[derive(Debug, Clone)]
pub struct PathPolicy {
    /// Directories the LLM is allowed to search / read.
    pub allowed_roots: Vec<PathBuf>,
    /// Directories that are always blocked (checked against canonical paths).
    pub denied_roots: Vec<PathBuf>,
    pub include_hidden_default: bool,
    /// Soft default cap for `read_text`.
    pub max_read_bytes: usize,
    /// Absolute hard cap regardless of request.
    pub hard_max_read_bytes: usize,
    pub max_search_results: usize,
}

impl PathPolicy {
    /// Build the default policy from the user's home directory.
    /// Skips missing roots instead of failing; never panics.
    pub fn default_for_user_home() -> Result<Self> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;

        let allowed_roots: Vec<PathBuf> =
            try_canonicalize(&home).map(|h| vec![h]).unwrap_or_default();

        // Denied roots — we want to block even non-existent dirs, so use
        // expanded absolute paths (no canonicalization required).
        let denied_candidates: Vec<PathBuf> = vec![
            home.join(".ssh"),
            home.join(".gnupg"),
            home.join(".aws"),
            home.join(".config/op"),
            home.join("Library/Keychains"),
            home.join("Library/Application Support/1Password"),
            home.join("Library/Application Support/com.1password"),
            home.join("Library/Application Support/Bitwarden"),
            home.join("Library/Application Support/Google/Chrome"),
            home.join("Library/Application Support/BraveSoftware"),
            home.join("Library/Application Support/Firefox/Profiles"),
            home.join("Library/Mobile Documents/com~apple~CloudDocs/.Trash"),
            home.join(".Trash"),
            PathBuf::from("/System"),
            PathBuf::from("/private/etc"),
            PathBuf::from("/private/var/db"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
            PathBuf::from("/sbin"),
            PathBuf::from("/usr/sbin"),
        ];

        let denied_roots: Vec<PathBuf> = denied_candidates
            .into_iter()
            .map(|p| try_canonicalize(&p).unwrap_or(p))
            .collect();

        Ok(Self {
            allowed_roots,
            denied_roots,
            include_hidden_default: false,
            max_read_bytes: 8_000,
            hard_max_read_bytes: 50_000,
            max_search_results: 50,
        })
    }

    /// Expand a raw path string: replace leading `~` with the home directory.
    pub fn expand_tilde(&self, raw: &str) -> Result<PathBuf> {
        if let Some(rest) = raw.strip_prefix("~/") {
            let home =
                dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;
            Ok(home.join(rest))
        } else if raw == "~" {
            dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))
        } else {
            Ok(PathBuf::from(raw))
        }
    }

    /// Validate a search root supplied by the user (or default allowed roots).
    pub fn validate_search_root(&self, raw: &str, include_hidden: bool) -> Result<PathBuf> {
        let expanded = self.expand_tilde(raw)?;
        let canonical = try_canonicalize(&expanded)?;
        if !canonical.exists() {
            return Err(anyhow!("path does not exist: {}", canonical.display()));
        }
        if self.is_denied(&canonical) {
            return Err(anyhow!("access denied: {}", canonical.display()));
        }
        if !self.is_under_allowed_root(&canonical) {
            return Err(anyhow!(
                "path outside allowed roots: {}",
                canonical.display()
            ));
        }
        if !include_hidden && self.contains_hidden_component(&canonical) {
            return Err(anyhow!("hidden path not allowed: {}", canonical.display()));
        }
        Ok(canonical)
    }

    /// Validate a path for `read_text`.
    pub fn validate_read_path(&self, raw: &str) -> Result<PathBuf> {
        let expanded = self.expand_tilde(raw)?;
        let canonical = std::fs::canonicalize(&expanded)
            .map_err(|e| anyhow!("cannot resolve path '{}': {}", expanded.display(), e))?;
        self.check_common(&canonical, false)?;
        if canonical.is_dir() {
            return Err(anyhow!("path is a directory, not a file"));
        }
        Ok(canonical)
    }

    /// Validate a path for `open_file` or `open_file_with`.
    pub fn validate_open_path(&self, raw: &str) -> Result<PathBuf> {
        let expanded = self.expand_tilde(raw)?;
        let canonical = std::fs::canonicalize(&expanded)
            .map_err(|e| anyhow!("cannot resolve path '{}': {}", expanded.display(), e))?;
        self.check_common(&canonical, false)?;
        if self.is_dangerous_executable(&canonical) {
            return Err(anyhow!(
                "opening executable/script/package files is not permitted: {}",
                canonical.display()
            ));
        }
        Ok(canonical)
    }

    /// Validate a path for `reveal_in_finder` or `open_folder`.
    pub fn validate_reveal_path(&self, raw: &str) -> Result<PathBuf> {
        let expanded = self.expand_tilde(raw)?;
        let canonical = std::fs::canonicalize(&expanded)
            .map_err(|e| anyhow!("cannot resolve path '{}': {}", expanded.display(), e))?;
        self.check_common(&canonical, false)?;
        Ok(canonical)
    }

    fn check_common(&self, canonical: &Path, include_hidden: bool) -> Result<()> {
        if self.is_denied(canonical) {
            return Err(anyhow!("access denied: {}", canonical.display()));
        }
        if !self.is_under_allowed_root(canonical) {
            return Err(anyhow!(
                "path outside allowed roots: {}",
                canonical.display()
            ));
        }
        if !include_hidden && self.contains_hidden_component(canonical) {
            return Err(anyhow!("hidden path not allowed: {}", canonical.display()));
        }
        Ok(())
    }

    /// Returns true if the canonical path is a prefix-descendant of any allowed root.
    pub fn is_under_allowed_root(&self, canonical: &Path) -> bool {
        self.allowed_roots.iter().any(|r| canonical.starts_with(r))
    }

    /// Returns true if the canonical path is under any denied root.
    pub fn is_denied(&self, canonical: &Path) -> bool {
        self.denied_roots.iter().any(|r| canonical.starts_with(r))
    }

    /// Returns true if any component of the path starts with `.`
    /// (after canonicalization, `.` and `..` are not present).
    pub fn contains_hidden_component(&self, canonical: &Path) -> bool {
        canonical.components().any(|c| {
            use std::path::Component;
            if let Component::Normal(s) = c {
                s.to_str().map(|s| s.starts_with('.')).unwrap_or(false)
            } else {
                false
            }
        })
    }

    /// Returns true if the file has a dangerous extension that should not be opened.
    pub fn is_dangerous_executable(&self, canonical: &Path) -> bool {
        if let Some(ext) = canonical.extension().and_then(|e| e.to_str()) {
            DANGEROUS_EXTENSIONS.contains(&ext.to_lowercase().as_str())
        } else {
            false
        }
    }
}

fn try_canonicalize(p: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(p).map_err(|e| anyhow!("cannot canonicalize '{}': {}", p.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn expands_tilde() {
        let policy = PathPolicy::default_for_user_home().unwrap();
        let home = dirs::home_dir().unwrap();
        let expanded = policy.expand_tilde("~/Documents").unwrap();
        assert_eq!(expanded, home.join("Documents"));
    }

    #[test]
    fn expands_bare_tilde() {
        let policy = PathPolicy::default_for_user_home().unwrap();
        let home = dirs::home_dir().unwrap();
        let expanded = policy.expand_tilde("~").unwrap();
        assert_eq!(expanded, home);
    }

    #[test]
    fn rejects_ssh_id_rsa() {
        let policy = PathPolicy::default_for_user_home().unwrap();
        let home = dirs::home_dir().unwrap();
        // Check that .ssh is denied even if the path exists
        let ssh = home.join(".ssh").join("id_rsa");
        // We check via is_denied on the expanded path
        let denied_path = home.join(".ssh");
        assert!(
            policy
                .denied_roots
                .iter()
                .any(|r| denied_path.starts_with(r) || r == &denied_path),
            "~/.ssh should be in denied roots"
        );
    }

    #[test]
    fn rejects_keychains() {
        let policy = PathPolicy::default_for_user_home().unwrap();
        let home = dirs::home_dir().unwrap();
        let keychains = home.join("Library/Keychains");
        assert!(
            policy
                .denied_roots
                .iter()
                .any(|r| keychains.starts_with(r) || r == &keychains),
            "~/Library/Keychains should be in denied roots"
        );
    }

    #[test]
    fn rejects_dangerous_executable_extensions() {
        let policy = PathPolicy::default_for_user_home().unwrap();
        assert!(policy.is_dangerous_executable(Path::new("/tmp/script.sh")));
        assert!(policy.is_dangerous_executable(Path::new("/tmp/app.pkg")));
        assert!(policy.is_dangerous_executable(Path::new("/tmp/Finder.app")));
        assert!(!policy.is_dangerous_executable(Path::new("/tmp/document.pdf")));
        assert!(!policy.is_dangerous_executable(Path::new("/tmp/notes.txt")));
    }

    #[test]
    fn detects_hidden_component() {
        let policy = PathPolicy::default_for_user_home().unwrap();
        assert!(policy.contains_hidden_component(Path::new("/Users/test/.hidden/file.txt")));
        assert!(!policy.contains_hidden_component(Path::new("/Users/test/Documents/file.txt")));
    }

    #[test]
    fn allows_normal_file_under_temp_allowed_root() {
        // Build a policy with a temp dir as the allowed root
        let tmp = std::env::temp_dir();
        let canonical_tmp = std::fs::canonicalize(&tmp).unwrap_or(tmp.clone());
        let test_file = tmp.join("bagent_test_allowed.txt");
        fs::write(&test_file, "hello").unwrap();

        let mut policy = PathPolicy::default_for_user_home().unwrap();
        policy.allowed_roots = vec![canonical_tmp.clone()];

        let canonical = std::fs::canonicalize(&test_file).unwrap();
        assert!(policy.is_under_allowed_root(&canonical));
        assert!(!policy.is_denied(&canonical));
        assert!(!policy.contains_hidden_component(&canonical));

        fs::remove_file(test_file).ok();
    }

    #[test]
    fn rejects_path_outside_allowed_roots() {
        let mut policy = PathPolicy::default_for_user_home().unwrap();
        // Replace allowed roots with an empty list
        policy.allowed_roots = vec![];
        let home = dirs::home_dir().unwrap();
        // Even a safe path is rejected when no allowed roots exist
        let p = home.join("Documents/test.txt");
        assert!(!policy.is_under_allowed_root(&p));
    }
}
