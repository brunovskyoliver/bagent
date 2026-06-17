//! Skill loader — scans directories for `SKILL.md` files and parses them.
//!
//! A skill directory looks like:
//!   `skills/sk-business-email/SKILL.md`
//!
//! The `SKILL.md` file starts with YAML frontmatter between `---` fences,
//! followed by the skill body (Markdown prose that gets injected into the prompt).
//!
//! Invalid frontmatter is logged and the skill is skipped — it must not crash
//! daemon startup.

use crate::manifest::{LoadedSkill, SkillManifest};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Scan `dir` for `*/SKILL.md` files and return parsed skills.
/// Errors for individual files are logged and skipped; the scan itself never fails.
pub fn scan_dir(dir: &Path) -> Vec<LoadedSkill> {
    let mut skills = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!("skills: scan_dir {}: {e}", dir.display());
            return skills;
        }
    };
    for entry in entries.flatten() {
        let skill_md = entry.path().join("SKILL.md");
        if skill_md.is_file() {
            match parse_skill_file(&skill_md) {
                Ok(skill) => {
                    tracing::debug!(
                        "skills: loaded '{}' from {}",
                        skill.manifest.name,
                        skill.source_path
                    );
                    skills.push(skill);
                }
                Err(e) => {
                    tracing::warn!("skills: skipping {}: {e}", skill_md.display());
                }
            }
        }
    }
    skills
}

/// Scan multiple directories. Skills in later dirs override earlier ones by name.
pub fn scan_dirs(dirs: &[PathBuf]) -> Vec<LoadedSkill> {
    let mut by_name: std::collections::HashMap<String, LoadedSkill> =
        std::collections::HashMap::new();
    for dir in dirs {
        for skill in scan_dir(dir) {
            by_name.insert(skill.manifest.name.clone(), skill);
        }
    }
    let mut skills: Vec<LoadedSkill> = by_name.into_values().collect();
    skills.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    skills
}

/// Parse a single `SKILL.md` file into a `LoadedSkill`.
pub fn parse_skill_file(path: &Path) -> Result<LoadedSkill> {
    let content = std::fs::read_to_string(path)?;
    parse_skill_content(&content, path.to_string_lossy().into_owned())
}

/// Parse skill content from a string (useful for tests).
pub fn parse_skill_content(content: &str, source_path: String) -> Result<LoadedSkill> {
    // Expect content to start with `---\n`, then YAML, then `---\n`, then body.
    let content = content.trim_start();
    let after_first = content
        .strip_prefix("---")
        .ok_or_else(|| anyhow::anyhow!("SKILL.md must start with '---' frontmatter"))?
        .trim_start_matches('\n');

    // Find the closing ---
    let close = after_first
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("SKILL.md frontmatter has no closing '---'"))?;

    let yaml_str = &after_first[..close];
    let body = after_first[close + 4..]
        .trim_start_matches('\n')
        .to_string();

    let manifest: SkillManifest = serde_yaml::from_str(yaml_str)
        .map_err(|e| anyhow::anyhow!("invalid SKILL.md frontmatter: {e}"))?;

    Ok(LoadedSkill {
        manifest,
        body,
        source_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SKILL: &str = r#"---
name: sk-business-email
description: Use when drafting Slovak business emails.
version: 1
risk: low
allowed_tools:
  - mail_get_message
  - memory_search
tags:
  - slovak
  - email
  - business
---

# Slovak Business Email Skill

Default style:
- formal Slovak
- preserve diacritics
- no Czech expressions
"#;

    #[test]
    fn parses_frontmatter() {
        let skill = parse_skill_content(SAMPLE_SKILL, "test".to_string()).unwrap();
        assert_eq!(skill.manifest.name, "sk-business-email");
        assert_eq!(skill.manifest.version, 1);
        assert!(skill.manifest.tags.contains(&"slovak".to_string()));
        assert!(skill
            .manifest
            .allowed_tools
            .contains(&"mail_get_message".to_string()));
        assert!(skill.body.contains("# Slovak Business Email Skill"));
    }

    #[test]
    fn parses_body_correctly() {
        let skill = parse_skill_content(SAMPLE_SKILL, "test".to_string()).unwrap();
        assert!(skill.body.contains("formal Slovak"));
        assert!(
            !skill.body.contains("---"),
            "frontmatter should not bleed into body"
        );
    }

    #[test]
    fn rejects_missing_frontmatter() {
        let bad = "# No frontmatter here\n\nJust a body.";
        assert!(parse_skill_content(bad, "test".to_string()).is_err());
    }

    #[test]
    fn scans_temp_dir() {
        let dir = std::env::temp_dir().join("bagent_skills_test");
        let skill_dir = dir.join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), SAMPLE_SKILL).unwrap();

        let skills = scan_dir(&dir);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].manifest.name, "sk-business-email");

        // cleanup
        std::fs::remove_dir_all(&dir).ok();
    }
}
