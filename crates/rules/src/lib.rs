use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalLevel {
    Auto,
    Ask,
    Forbidden,
}

impl Default for ApprovalLevel {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Exact tool name to match; None = match all tools.
    pub tool: Option<String>,
    /// Regex applied to JSON-serialized args; None = match any args.
    pub args_pattern: Option<String>,
    pub level: ApprovalLevel,
}

impl Rule {
    fn matches(&self, tool: &str, args_json: &str) -> bool {
        if let Some(t) = &self.tool {
            if t.as_str() != tool {
                return false;
            }
        }
        if let Some(pat) = &self.args_pattern {
            match Regex::new(pat) {
                Ok(re) => {
                    if !re.is_match(args_json) {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        }
        true
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct RuleSetFile {
    version: u32,
    rules: Vec<Rule>,
}

pub struct RuleEngine {
    rules: Arc<RwLock<Vec<Rule>>>,
    path: Option<PathBuf>,
}

impl RuleEngine {
    pub fn with_default() -> Self {
        Self {
            rules: Arc::new(RwLock::new(default_rules())),
            path: None,
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let parsed: RuleSetFile = serde_yaml::from_str(&content)?;
        Ok(Self {
            rules: Arc::new(RwLock::new(parsed.rules)),
            path: Some(path.to_path_buf()),
        })
    }

    pub fn load_or_default(path: &Path) -> Self {
        match Self::load(path) {
            Ok(engine) => engine,
            Err(e) => {
                tracing::warn!("failed to load rules.yaml ({e}), using defaults");
                let mut engine = Self::with_default();
                engine.path = Some(path.to_path_buf());
                engine
            }
        }
    }

    /// Spawn a background task that polls the rules file every 5 s and reloads on change.
    pub fn spawn_hot_reload(self: Arc<Self>) {
        let rules_arc = self.rules.clone();
        let path = match &self.path {
            Some(p) => p.clone(),
            None => return,
        };
        tokio::spawn(async move {
            use tokio::time::{Duration, interval};
            let mut last_mtime: Option<std::time::SystemTime> = None;
            let mut ticker = interval(Duration::from_secs(5));
            loop {
                ticker.tick().await;
                if let Ok(meta) = tokio::fs::metadata(&path).await {
                    let mtime = meta.modified().ok();
                    if mtime != last_mtime && last_mtime.is_some() {
                        match tokio::fs::read_to_string(&path).await {
                            Ok(content) => {
                                if let Ok(parsed) = serde_yaml::from_str::<RuleSetFile>(&content) {
                                    *rules_arc.write().unwrap() = parsed.rules;
                                    tracing::info!("rules reloaded from {}", path.display());
                                }
                            }
                            Err(e) => tracing::warn!("rules reload error: {e}"),
                        }
                    }
                    last_mtime = mtime;
                }
            }
        });
    }

    /// First matching rule wins; default is Auto.
    pub fn check(&self, tool: &str, args_json: &str) -> ApprovalLevel {
        let rules = self.rules.read().unwrap();
        for rule in rules.iter() {
            if rule.matches(tool, args_json) {
                return rule.level.clone();
            }
        }
        ApprovalLevel::Auto
    }

    pub fn rules_yaml(&self) -> String {
        let rules = self.rules.read().unwrap().clone();
        let set = RuleSetFile { version: 1, rules };
        serde_yaml::to_string(&set).unwrap_or_default()
    }

    /// Validate YAML, apply in-memory, persist to disk.
    pub fn save_yaml(&self, content: &str) -> Result<()> {
        let parsed: RuleSetFile = serde_yaml::from_str(content)?;
        *self.rules.write().unwrap() = parsed.rules;
        if let Some(ref path) = self.path {
            std::fs::write(path, content)?;
        }
        Ok(())
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

fn default_rules() -> Vec<Rule> {
    vec![
        Rule {
            name: "auto_read_mail".into(),
            description: Some("Reading mail inbox proceeds automatically".into()),
            tool: Some("mail_inbox".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
        Rule {
            name: "auto_read_notes".into(),
            description: Some("Reading notes proceeds automatically".into()),
            tool: Some("notes_list".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
        Rule {
            name: "auto_notes_search".into(),
            description: None,
            tool: Some("notes_search".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
    ]
}

pub const DEFAULT_RULES_YAML: &str = r#"version: 1
# Rules are evaluated top-to-bottom; first match wins.
# Levels:
#   auto      — proceed without asking
#   ask       — show approval modal before proceeding
#   forbidden — always block

rules:
  - name: auto_read_mail
    description: "Reading mail inbox proceeds automatically"
    tool: mail_inbox
    level: auto

  - name: auto_read_notes
    description: "Reading notes proceeds automatically"
    tool: notes_list
    level: auto

  - name: auto_notes_search
    tool: notes_search
    level: auto
"#;
