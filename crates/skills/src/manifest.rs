//! Skill manifest — parsed from the YAML frontmatter of a `SKILL.md` file.

use serde::{Deserialize, Serialize};

/// Risk level of a skill (informational — does not bypass the rules engine).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
}

/// Parsed frontmatter from a `SKILL.md` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub risk: RiskLevel,
    /// Tool names the skill may use — descriptive only; rules engine remains authority.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_version() -> u32 {
    1
}

/// A fully loaded skill — manifest + body text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedSkill {
    pub manifest: SkillManifest,
    /// Full body of the `SKILL.md` below the frontmatter block.
    pub body: String,
    /// Where the file was loaded from (for debug traces).
    pub source_path: String,
}
