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
            use tokio::time::{interval, Duration};
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
        // ── Phase 13A: Filesystem / app-open tools ────────────────────────────
        Rule {
            name: "auto_fs_search_files".into(),
            description: Some("Filename/path search runs automatically".into()),
            tool: Some("filesystem.search_files".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
        Rule {
            name: "auto_fs_search_content".into(),
            description: Some("Content search runs automatically".into()),
            tool: Some("filesystem.search_content".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
        Rule {
            name: "auto_fs_read_text".into(),
            description: Some("Reading a small text snippet runs automatically".into()),
            tool: Some("filesystem.read_text".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
        Rule {
            name: "auto_fs_metadata".into(),
            description: None,
            tool: Some("filesystem.metadata".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
        Rule {
            name: "auto_fs_reveal_in_finder".into(),
            description: Some("Revealing a file in Finder runs automatically".into()),
            tool: Some("filesystem.reveal_in_finder".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
        Rule {
            name: "auto_fs_open_folder".into(),
            description: Some("Opening a folder runs automatically".into()),
            tool: Some("filesystem.open_folder".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
        Rule {
            name: "ask_fs_open_file".into(),
            description: Some("Opening a file requires approval".into()),
            tool: Some("filesystem.open_file".into()),
            args_pattern: None,
            level: ApprovalLevel::Ask,
        },
        Rule {
            name: "ask_fs_open_file_with".into(),
            description: Some("Opening a file with a specific app requires approval".into()),
            tool: Some("filesystem.open_file_with".into()),
            args_pattern: None,
            level: ApprovalLevel::Ask,
        },
        Rule {
            name: "auto_macos_open_app".into(),
            description: Some("Launching an app runs automatically".into()),
            tool: Some("macos.open_app".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
        Rule {
            name: "auto_macos_focus_app".into(),
            description: Some("Focusing an app runs automatically".into()),
            tool: Some("macos.focus_app".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
        Rule {
            name: "forbidden_shell_exec".into(),
            description: Some("Arbitrary shell execution is always forbidden".into()),
            tool: Some("macos.shell_exec".into()),
            args_pattern: None,
            level: ApprovalLevel::Forbidden,
        },
        Rule {
            name: "forbidden_applescript_run".into(),
            description: Some("Arbitrary AppleScript execution is always forbidden".into()),
            tool: Some("macos.applescript_run".into()),
            args_pattern: None,
            level: ApprovalLevel::Forbidden,
        },
        // ── Phase 8: Codex external-reasoning harness ─────────────────────────
        Rule {
            name: "ask_codex_run_task".into(),
            description: Some(
                "Dispatching a task to the Codex external-reasoning harness always \
                 requires explicit user approval — Codex is external and receives a \
                 daemon-built context packet."
                    .into(),
            ),
            tool: Some("codex.run_task".into()),
            args_pattern: None,
            level: ApprovalLevel::Ask,
        },
        // ── Phase 6: Odoo connector — write-side guard ────────────────────────
        Rule {
            name: "forbidden_odoo_create_record".into(),
            description: Some("Creating Odoo records is always forbidden".into()),
            tool: Some("odoo.create_record".into()),
            args_pattern: None,
            level: ApprovalLevel::Forbidden,
        },
        Rule {
            name: "forbidden_odoo_write_record".into(),
            description: Some("Updating Odoo records is always forbidden".into()),
            tool: Some("odoo.write_record".into()),
            args_pattern: None,
            level: ApprovalLevel::Forbidden,
        },
        Rule {
            name: "forbidden_odoo_unlink_record".into(),
            description: Some("Deleting Odoo records is always forbidden".into()),
            tool: Some("odoo.unlink_record".into()),
            args_pattern: None,
            level: ApprovalLevel::Forbidden,
        },
        Rule {
            name: "forbidden_odoo_send_email".into(),
            description: Some("Sending email from Odoo is always forbidden".into()),
            tool: Some("odoo.send_email".into()),
            args_pattern: None,
            level: ApprovalLevel::Forbidden,
        },
        // ── Phase 11: WhatsApp connector ──────────────────────────────────────
        // NOTE: The send rule here is *cosmetic* documentation only.
        // The enforcement floor lives in the daemon send route, which always
        // calls request_approval_core regardless of rules.yaml state.
        // (Existing installs already have rules.yaml on disk; adding it here
        // ensures fresh installs and in-memory defaults are consistent.)
        Rule {
            name: "ask_whatsapp_send_message".into(),
            description: Some(
                "Sending a WhatsApp message always requires explicit user approval \
                 — one approval per message, no bulk, no auto-reply."
                    .into(),
            ),
            tool: Some("whatsapp.send_message".into()),
            args_pattern: None,
            level: ApprovalLevel::Ask,
        },
        Rule {
            name: "auto_whatsapp_list_chats".into(),
            description: Some("Listing recent WhatsApp chats runs automatically".into()),
            tool: Some("whatsapp.list_chats".into()),
            args_pattern: None,
            level: ApprovalLevel::Auto,
        },
        Rule {
            name: "auto_whatsapp_search_messages".into(),
            description: Some("Searching WhatsApp message cache runs automatically".into()),
            tool: Some("whatsapp.search_messages".into()),
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

  # Phase 13A — Filesystem / app-open tools
  - name: auto_fs_search_files
    description: "Filename/path search runs automatically"
    tool: filesystem.search_files
    level: auto

  - name: auto_fs_search_content
    description: "Content search runs automatically"
    tool: filesystem.search_content
    level: auto

  - name: auto_fs_read_text
    description: "Reading a small text snippet runs automatically"
    tool: filesystem.read_text
    level: auto

  - name: auto_fs_metadata
    tool: filesystem.metadata
    level: auto

  - name: auto_fs_reveal_in_finder
    description: "Revealing a file in Finder runs automatically"
    tool: filesystem.reveal_in_finder
    level: auto

  - name: auto_fs_open_folder
    description: "Opening a folder runs automatically"
    tool: filesystem.open_folder
    level: auto

  - name: ask_fs_open_file
    description: "Opening a file requires approval"
    tool: filesystem.open_file
    level: ask

  - name: ask_fs_open_file_with
    description: "Opening a file with a specific app requires approval"
    tool: filesystem.open_file_with
    level: ask

  - name: auto_macos_open_app
    description: "Launching an app runs automatically"
    tool: macos.open_app
    level: auto

  - name: auto_macos_focus_app
    description: "Focusing an app runs automatically"
    tool: macos.focus_app
    level: auto

  - name: forbidden_shell_exec
    description: "Arbitrary shell execution is always forbidden"
    tool: macos.shell_exec
    level: forbidden

  - name: forbidden_applescript_run
    description: "Arbitrary AppleScript execution is always forbidden"
    tool: macos.applescript_run
    level: forbidden

  # Phase 8 — Codex external-reasoning harness
  - name: ask_codex_run_task
    description: "Dispatching to the Codex external-reasoning harness always requires approval"
    tool: codex.run_task
    level: ask

  # Phase 6 — Odoo connector write-side guard
  - name: forbidden_odoo_create_record
    description: "Creating Odoo records is always forbidden"
    tool: odoo.create_record
    level: forbidden

  - name: forbidden_odoo_write_record
    description: "Updating Odoo records is always forbidden"
    tool: odoo.write_record
    level: forbidden

  - name: forbidden_odoo_unlink_record
    description: "Deleting Odoo records is always forbidden"
    tool: odoo.unlink_record
    level: forbidden

  - name: forbidden_odoo_send_email
    description: "Sending email from Odoo is always forbidden"
    tool: odoo.send_email
    level: forbidden

  # Phase 11 — WhatsApp connector
  # NOTE: The send rule is cosmetic documentation; enforcement is in the daemon route.
  - name: ask_whatsapp_send_message
    description: "Sending a WhatsApp message always requires explicit user approval — one per message, no bulk, no auto-reply"
    tool: whatsapp.send_message
    level: ask

  - name: auto_whatsapp_list_chats
    description: "Listing recent WhatsApp chats runs automatically"
    tool: whatsapp.list_chats
    level: auto

  - name: auto_whatsapp_search_messages
    description: "Searching WhatsApp message cache runs automatically"
    tool: whatsapp.search_messages
    level: auto
"#;
