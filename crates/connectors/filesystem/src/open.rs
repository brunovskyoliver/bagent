use anyhow::{anyhow, Result};
use tokio::process::Command;

use crate::path_policy::PathPolicy;
use crate::types::OpenResponse;

/// Apps that are explicitly pre-approved for `open_file_with` / `open_app`.
const COMMON_ALLOWED_APPS: &[&str] = &[
    "Finder",
    "Preview",
    "TextEdit",
    "Pages",
    "Numbers",
    "Keynote",
    "Microsoft Word",
    "Microsoft Excel",
    "Microsoft PowerPoint",
    "Visual Studio Code",
    "Cursor",
    "Xcode",
    "Safari",
    "Google Chrome",
    "Mail",
    "Calendar",
    "Notes",
];

/// Characters not allowed in an app name.
const FORBIDDEN_APP_CHARS: &[char] = &['/', '\\', ';', '&', '|', '$', '`', '<', '>', '"', '\''];

// ── Pure argv builders (no process launch) — test-safe ───────────────────────

/// Build argv for: reveal file in Finder (`open -R <path>`)
pub fn build_reveal_argv(path: &str) -> Vec<String> {
    vec!["/usr/bin/open".into(), "-R".into(), path.into()]
}

/// Build argv for: open folder (`open <folder>`)
pub fn build_open_folder_argv(path: &str) -> Vec<String> {
    vec!["/usr/bin/open".into(), path.into()]
}

/// Build argv for: open file with default app (`open <file>`)
pub fn build_open_file_argv(path: &str) -> Vec<String> {
    vec!["/usr/bin/open".into(), path.into()]
}

/// Build argv for: open file with specific app (`open -a <App> <file>`)
pub fn build_open_file_with_argv(path: &str, app: &str) -> Vec<String> {
    vec!["/usr/bin/open".into(), "-a".into(), app.into(), path.into()]
}

/// Build argv for: open/launch/focus an app (`open -a <App>`)
pub fn build_open_app_argv(app: &str) -> Vec<String> {
    vec!["/usr/bin/open".into(), "-a".into(), app.into()]
}

/// Build argv for: open a URL in Safari (`open -a Safari <url>`)
pub fn build_open_url_argv(url: &str) -> Vec<String> {
    vec![
        "/usr/bin/open".into(),
        "-a".into(),
        "Safari".into(),
        url.into(),
    ]
}

// ── Validation ────────────────────────────────────────────────────────────────

/// Validate an app name: non-empty, only safe characters, not a path.
pub fn validate_app_name(app: &str) -> Result<()> {
    let trimmed = app.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("app name cannot be empty"));
    }
    if trimmed.len() > 128 {
        return Err(anyhow!("app name too long"));
    }
    for ch in trimmed.chars() {
        if FORBIDDEN_APP_CHARS.contains(&ch) {
            return Err(anyhow!("app name contains forbidden character: {ch:?}"));
        }
        if !ch.is_alphanumeric() && ch != ' ' && ch != '.' && ch != '-' && ch != '_' {
            return Err(anyhow!("app name contains invalid character: {ch:?}"));
        }
    }
    Ok(())
}

// ── Execution (async, macOS-only) ─────────────────────────────────────────────

async fn run_argv(argv: &[String]) -> Result<()> {
    let (bin, args) = argv.split_first().ok_or_else(|| anyhow!("empty argv"))?;
    let status = Command::new(bin).args(args).status().await?;
    if !status.success() {
        return Err(anyhow!("command exited with status {}", status));
    }
    Ok(())
}

/// Reveal a file in Finder: `open -R <path>`.
pub async fn reveal_in_finder(policy: &PathPolicy, raw_path: &str) -> Result<OpenResponse> {
    let canonical = policy.validate_reveal_path(raw_path)?;
    let path_str = canonical.to_string_lossy().into_owned();
    let argv = build_reveal_argv(&path_str);
    run_argv(&argv).await?;
    Ok(OpenResponse {
        ok: true,
        path: Some(path_str),
        app: None,
        action: "reveal_in_finder".into(),
    })
}

/// Open a folder in Finder: `open <folder>`.
pub async fn open_folder(policy: &PathPolicy, raw_path: &str) -> Result<OpenResponse> {
    let canonical = policy.validate_reveal_path(raw_path)?;
    if !canonical.is_dir() {
        return Err(anyhow!("not a directory: {}", canonical.display()));
    }
    let path_str = canonical.to_string_lossy().into_owned();
    let argv = build_open_folder_argv(&path_str);
    run_argv(&argv).await?;
    Ok(OpenResponse {
        ok: true,
        path: Some(path_str),
        app: None,
        action: "open_folder".into(),
    })
}

/// Open a file with its default app: `open <file>`.
pub async fn open_file(policy: &PathPolicy, raw_path: &str) -> Result<OpenResponse> {
    let canonical = policy.validate_open_path(raw_path)?;
    let path_str = canonical.to_string_lossy().into_owned();
    let argv = build_open_file_argv(&path_str);
    run_argv(&argv).await?;
    Ok(OpenResponse {
        ok: true,
        path: Some(path_str),
        app: None,
        action: "open_file".into(),
    })
}

/// Open a file with a specific app: `open -a <App> <file>`.
pub async fn open_file_with(
    policy: &PathPolicy,
    raw_path: &str,
    app: &str,
) -> Result<OpenResponse> {
    validate_app_name(app)?;
    let canonical = policy.validate_open_path(raw_path)?;
    let path_str = canonical.to_string_lossy().into_owned();
    let argv = build_open_file_with_argv(&path_str, app.trim());
    run_argv(&argv).await?;
    Ok(OpenResponse {
        ok: true,
        path: Some(path_str),
        app: Some(app.trim().to_string()),
        action: "open_file_with".into(),
    })
}

/// Launch or focus an app: `open -a <App>`.
/// Works for both common and unknown app names (name is validated but not restricted to list).
pub async fn open_app(app: &str) -> Result<OpenResponse> {
    validate_app_name(app)?;
    let argv = build_open_app_argv(app.trim());
    run_argv(&argv).await?;
    Ok(OpenResponse {
        ok: true,
        path: None,
        app: Some(app.trim().to_string()),
        action: "open_app".into(),
    })
}

/// Open a URL in Safari: `open -a Safari <url>`.
///
/// Deliberately bypasses `PathPolicy` — URLs are not file paths. Only `http(s)://`
/// schemes are accepted; any other scheme (file://, javascript:, …) is rejected.
pub async fn open_url_in_safari(url: &str) -> Result<()> {
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(anyhow!(
            "open_url_in_safari: only http/https URLs are allowed, got: {trimmed:?}"
        ));
    }
    let argv = build_open_url_argv(trimmed);
    run_argv(&argv).await
}

/// Alias for `open_app` — `open -a <App>` brings the app to the foreground.
pub async fn focus_app(app: &str) -> Result<OpenResponse> {
    validate_app_name(app)?;
    let argv = build_open_app_argv(app.trim());
    run_argv(&argv).await?;
    Ok(OpenResponse {
        ok: true,
        path: None,
        app: Some(app.trim().to_string()),
        action: "focus_app".into(),
    })
}

// ── Tests (pure argv, no process launch) ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_reveal() {
        let argv = build_reveal_argv("/Users/me/file.pdf");
        assert_eq!(argv, vec!["/usr/bin/open", "-R", "/Users/me/file.pdf"]);
    }

    #[test]
    fn argv_open_folder() {
        let argv = build_open_folder_argv("/Users/me/Documents");
        assert_eq!(argv, vec!["/usr/bin/open", "/Users/me/Documents"]);
    }

    #[test]
    fn argv_open_file() {
        let argv = build_open_file_argv("/Users/me/doc.pdf");
        assert_eq!(argv, vec!["/usr/bin/open", "/Users/me/doc.pdf"]);
    }

    #[test]
    fn argv_open_file_with_preview() {
        let argv = build_open_file_with_argv("/Users/me/doc.pdf", "Preview");
        assert_eq!(
            argv,
            vec!["/usr/bin/open", "-a", "Preview", "/Users/me/doc.pdf"]
        );
    }

    #[test]
    fn argv_open_app_mail() {
        let argv = build_open_app_argv("Mail");
        assert_eq!(argv, vec!["/usr/bin/open", "-a", "Mail"]);
    }

    #[test]
    fn argv_open_app_excel() {
        let argv = build_open_app_argv("Microsoft Excel");
        assert_eq!(argv, vec!["/usr/bin/open", "-a", "Microsoft Excel"]);
    }

    #[test]
    fn validate_app_name_ok() {
        assert!(validate_app_name("Preview").is_ok());
        assert!(validate_app_name("Microsoft Excel").is_ok());
        assert!(validate_app_name("Visual Studio Code").is_ok());
        assert!(validate_app_name("Xcode").is_ok());
    }

    #[test]
    fn validate_app_name_forbidden_chars() {
        assert!(validate_app_name("evil; rm -rf /").is_err());
        assert!(validate_app_name("app|evil").is_err());
        assert!(validate_app_name("app/path").is_err());
        assert!(validate_app_name("app`cmd`").is_err());
        assert!(validate_app_name("app\"name\"").is_err());
    }

    #[test]
    fn validate_app_name_empty() {
        assert!(validate_app_name("").is_err());
        assert!(validate_app_name("   ").is_err());
    }

    #[test]
    fn validate_app_name_too_long() {
        let long = "a".repeat(200);
        assert!(validate_app_name(&long).is_err());
    }

    #[test]
    fn argv_open_url_safari() {
        let argv = build_open_url_argv("https://myco.odoo.com/web#id=42&model=res.partner");
        assert_eq!(
            argv,
            vec![
                "/usr/bin/open",
                "-a",
                "Safari",
                "https://myco.odoo.com/web#id=42&model=res.partner"
            ]
        );
    }

    #[tokio::test]
    async fn open_url_rejects_non_http() {
        assert!(open_url_in_safari("file:///etc/passwd").await.is_err());
        assert!(open_url_in_safari("javascript:alert(1)").await.is_err());
        assert!(open_url_in_safari("ftp://example.com/file").await.is_err());
        assert!(open_url_in_safari("not-a-url").await.is_err());
    }
}
