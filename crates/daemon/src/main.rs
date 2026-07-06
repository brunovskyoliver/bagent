use anyhow::Result;
use apple_mail_connector::MailSearchFilter;
use apple_mail_connector::{self, MailConnector};
use apple_notes_connector::NotesConnector;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{delete, get, post},
    Json, Router,
};
use bagent_agent::{PromptBuilder, PromptTrace, ScreenIntentClassifier, SelectedSkill, TaskRater};
use bagent_attachments::extract as extract_attachment;
use bagent_memory::MemoryStore;
use bagent_rules::{ApprovalLevel, RuleEngine, DEFAULT_RULES_YAML};
use bagent_skills::{selector as skill_selector, LoadedSkill};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use codex_connector::{
    CodexConfig, CodexConnector, CodexContextPacket, CodexExpectedOutput, CodexTask, ContextItem,
};
use filesystem_connector::{
    self, open as fs_open, search as fs_search, FileSearchRequest, FsConnector, OpenResponse,
    ReadTextRequest,
};
use futures_util::StreamExt;
use odoo_connector::{OdooConfig, OdooConnector, OdooError, OdooRecordRef};
use ollama_connector::{Message, OllamaClient, DEFAULT_BASE_URL, DEFAULT_EMBED_MODEL};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    io::Write,
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use whatsapp_connector::{
    WhatsappConfig, WhatsappConnectionStatus, WhatsappConnector, WhatsappSendTarget,
};

mod embedded {
    refinery::embed_migrations!("migrations");
}

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    db_path: PathBuf,
    token: String,
    default_model: String,
    debug_dir: PathBuf,
    /// Small fast model for intent/correction classifiers — never blocks chat TTFT.
    classifier_model: String,
    vision_model: String,
    attachments_dir: PathBuf,
    ollama: OllamaClient,
    mail: Option<MailConnector>,
    notes: Option<NotesConnector>,
    fs: Option<FsConnector>,
    memory: Arc<MemoryStore>,
    prompt_builder: Arc<PromptBuilder>,
    rules: Arc<RuleEngine>,
    pending_approvals: Arc<std::sync::Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    /// Loaded skill manifests + bodies, scanned at startup.
    skills: Arc<Vec<LoadedSkill>>,
    /// Deterministic task rater — classifies local vs Codex tasks.
    task_rater: Arc<TaskRater>,
    /// Codex external-reasoning connector (None when binary not found).
    codex: Option<CodexConnector>,
    /// Odoo connector — in-memory only; API key never written to disk.
    /// Swift pushes credentials from Keychain lazily when an Odoo turn needs it.
    odoo: Arc<RwLock<Option<OdooConnector>>>,
    /// WhatsApp Web bridge connector. Always present; owns the bridge subprocess.
    /// Bridge can autostart when a prior LocalAuth session exists, and is also
    /// controlled explicitly via `/whatsapp/start` and `/whatsapp/stop`.
    whatsapp: Arc<WhatsappConnector>,
    /// Ephemeral connector refs for current daemon run only. Never persisted.
    runtime_refs: Arc<Mutex<HashMap<String, RuntimeRefs>>>,
}

#[derive(Debug, Clone, Default)]
struct RuntimeRefs {
    mail: Option<MailRef>,
    file: Option<FileRef>,
    odoo: Option<OdooRecordRef>,
    whatsapp: Option<WhatsappRef>,
}

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
    #[serde(default)]
    #[allow(dead_code)]
    history: Vec<Message>,
    model: Option<String>,
    session_id: Option<String>,
    /// IDs returned by POST /attachments — empty when no files attached.
    #[serde(default)]
    attachment_ids: Vec<String>,
    // ── Screen context (Phase 7) ─────────────────────────────────────────────
    /// Base64-encoded PNG of the user's screen captured in Swift.
    /// Never persisted to disk — injected into the model turn in-memory only.
    #[serde(default)]
    screen_image_b64: Option<String>,
    /// On-device OCR text extracted from the captured frame (Vision framework).
    #[serde(default)]
    screen_ocr_text: Option<String>,
    /// Frontmost application name + bundle id at capture time.
    #[serde(default)]
    active_app: Option<String>,
    /// Accessibility selected-text at capture time (password fields excluded).
    #[serde(default)]
    selected_text: Option<String>,
    /// Optional UI-selected source mode from the Spotlight-like input.
    /// Values: mail, filesystem, whatsapp, odoo.
    #[serde(default)]
    source_mode: Option<String>,
}

#[derive(Serialize)]
struct PromptDebugRecord {
    prompt_trace_id: String,
    session_id: String,
    created_at: String,
    user_message: String,
    model: String,
    language: String,
    prompt_chars: usize,
    prompt_token_estimate: usize,
    message_count: usize,
    prompt_messages: Vec<PromptDebugMessage>,
    trace: PromptTrace,
    response_preview: String,
    response_chars: usize,
    elapsed_ms: u128,
}

#[derive(Serialize)]
struct PromptDebugMessage {
    role: String,
    content: String,
    images_count: usize,
}

#[derive(Deserialize)]
struct ApprovalDecideRequest {
    allow: bool,
}

#[derive(Deserialize)]
struct RulesSaveRequest {
    yaml: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct MemoryInsertRequest {
    namespace: String,
    kind: String,
    #[serde(default = "default_und")]
    language: String,
    text: String,
    source_ref: Option<String>,
    metadata_json: Option<String>,
    expires_at: Option<String>,
    // V11 ledger fields
    confidence: Option<f32>,
    importance: Option<f32>,
    source: Option<String>,
    sensitivity: Option<String>,
    subject: Option<String>,
}

fn default_und() -> String {
    "und".to_string()
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct MemorySearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    namespace: String,
    #[serde(default = "default_limit")]
    limit: usize,
    // V11 filter: empty string = all kinds
    #[serde(default)]
    kind: String,
}

#[derive(Deserialize)]
struct EmbedRequest {
    input: String,
    model: Option<String>,
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    unread: bool,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    20
}

/// Stable reference to a found mail message — surfaced to the frontend so
/// Stable reference to the most recently found local file/folder, persisted in
/// `sessions.metadata_json` so cross-turn references ("open it", "otvor ho") resolve correctly.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileRef {
    path: String,
    display_name: String,
    kind: String,
}

/// it can render an "Otvoriť mail" button without re-running a search.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MailRef {
    rowid: i64,
    message_id: Option<String>,
    subject: String,
    sender: String,
    /// When true the Swift client should auto-open Mail.app after the first
    /// sentence of the LLM response has streamed in.
    auto_open: bool,
}






#[derive(Debug, Clone, Serialize, Deserialize)]
struct WhatsappRef {
    chat_id: String,
    contact_name: Option<String>,
    snippet: Option<String>,
    source: String,
    #[serde(default)]
    last_message_timestamp: Option<i64>,
}

/// Request body for `POST /mail/open`.
#[derive(Deserialize)]
struct MailOpenReq {
    rowid: Option<i64>,
    message_id: Option<String>,
    subject: String,
    sender: String,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    ollama: bool,
    model: String,
    classifier_model: String,
    connectors: ConnectorStatus,
}

#[derive(Serialize)]
struct ConnectorStatus {
    mail: bool,
    notes: bool,
    odoo: bool,
    whatsapp: WhatsappHealthStatus,
}

#[derive(Serialize)]
struct WhatsappHealthStatus {
    status: String,
    connected: bool,
    needs_qr: bool,
    error: Option<String>,
}

async fn shutdown_signal(state: AppState) {
    wait_for_shutdown_signal().await;
    tracing::info!("shutdown signal received; cleaning runtime resources");
    cleanup_runtime_resources(&state).await;
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => {}
        _ = sigint.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn cleanup_runtime_resources(state: &AppState) {
    cleanup_ollama_models(state).await;

    if let Err(e) = state.whatsapp.stop().await {
        tracing::debug!("shutdown: WhatsApp stop skipped: {e}");
    }
}

async fn cleanup_ollama_models(state: &AppState) {
    let loaded_models = match state.ollama.loaded_models().await {
        Ok(models) => models,
        Err(e) => {
            tracing::debug!("shutdown: Ollama loaded-model check skipped: {e}");
            return;
        }
    };

    let mut seen = HashSet::new();
    let mut generate_models = Vec::new();
    for model in [
        state.default_model.as_str(),
        state.classifier_model.as_str(),
        state.vision_model.as_str(),
    ] {
        let trimmed = model.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            if let Some(loaded_name) = matching_loaded_model(&loaded_models, trimmed) {
                generate_models.push(loaded_name);
            }
        }
    }

    for model in generate_models {
        match state.ollama.unload_generate_model(&model).await {
            Ok(()) => tracing::info!(model = %model, "shutdown: Ollama model unloaded"),
            Err(e) => tracing::debug!(model = %model, "shutdown: Ollama model unload skipped: {e}"),
        }
    }

    if let Some(loaded_embed_model) = matching_loaded_model(&loaded_models, DEFAULT_EMBED_MODEL) {
        match state
            .ollama
            .unload_embedding_model(&loaded_embed_model)
            .await
        {
            Ok(()) => tracing::info!(
                model = %loaded_embed_model,
                "shutdown: Ollama embedding model unloaded"
            ),
            Err(e) => tracing::debug!(
                model = %loaded_embed_model,
                "shutdown: Ollama embedding unload skipped: {e}"
            ),
        }
    }
}

fn matching_loaded_model(loaded_models: &[String], requested: &str) -> Option<String> {
    let requested = requested.trim();
    if requested.is_empty() {
        return None;
    }
    loaded_models.iter().find_map(|loaded| {
        let loaded = loaded.trim();
        let exact = loaded == requested;
        let requested_latest = !requested.contains(':') && loaded == format!("{requested}:latest");
        let loaded_latest = loaded
            .strip_suffix(":latest")
            .map(|base| base == requested)
            .unwrap_or(false);
        let requested_latest_alias = requested
            .strip_suffix(":latest")
            .map(|base| base == loaded)
            .unwrap_or(false);
        if exact || requested_latest || loaded_latest || requested_latest_alias {
            Some(loaded.to_string())
        } else {
            None
        }
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let data_dir = app_data_dir();
    std::fs::create_dir_all(&data_dir)?;
    let attachments_dir = data_dir.join("attachments");
    std::fs::create_dir_all(&attachments_dir)?;
    let debug_dir = data_dir.join("debug");
    std::fs::create_dir_all(&debug_dir)?;
    std::fs::write(data_dir.join("daemon.pid"), std::process::id().to_string())?;

    let mut conn = Connection::open(data_dir.join("bagent.db"))?;
    embedded::migrations::runner()
        .run(&mut conn)
        .map_err(|e| anyhow::anyhow!("migration error: {e}"))?;
    purge_legacy_context_data(&data_dir, &mut conn);
    let db = Arc::new(Mutex::new(conn));

    let token_path = data_dir.join("daemon.token");
    let token = if token_path.exists() {
        std::fs::read_to_string(&token_path)?.trim().to_string()
    } else {
        let t = Uuid::new_v4().to_string();
        std::fs::write(&token_path, &t)?;
        t
    };

    let mail = MailConnector::new().ok().filter(|c| c.is_accessible());
    let notes = NotesConnector::new().ok().filter(|c| c.is_accessible());
    let fs = FsConnector::new().ok().filter(|c| c.is_accessible());

    if mail.is_some() {
        tracing::info!("Mail connector: accessible");
    } else {
        tracing::warn!("Mail connector: no Full Disk Access");
    }
    if notes.is_some() {
        tracing::info!("Notes connector: accessible");
    } else {
        tracing::warn!("Notes connector: no Full Disk Access");
    }
    if fs.is_some() {
        tracing::info!("Filesystem connector: accessible");
    } else {
        tracing::warn!("Filesystem connector: could not build default policy");
    }

    let ollama = OllamaClient::new(DEFAULT_BASE_URL);

    // MemoryStore uses a separate connection with std::sync::Mutex (blocking SQLite ops)
    let mem_conn = rusqlite::Connection::open(data_dir.join("bagent.db"))?;
    let mem_db = Arc::new(std::sync::Mutex::new(mem_conn));
    let memory = Arc::new(MemoryStore::new(mem_db, ollama.clone()).with_data_dir(data_dir.clone()));
    let prompt_builder = Arc::new(PromptBuilder::new());

    let default_model =
        std::env::var("BAGENT_DEFAULT_MODEL").unwrap_or_else(|_| "qwen3:8b".to_string());
    let classifier_model =
        std::env::var("BAGENT_CLASSIFIER_MODEL").unwrap_or_else(|_| "qwen3:0.6b".to_string());
    let vision_model =
        std::env::var("BAGENT_VISION_MODEL").unwrap_or_else(|_| "qwen2.5vl:7b".to_string());

    // Startup: warm the selected chat model and embed model into memory so first
    // user query doesn't pay cold-load cost. Vision stays lazy and only loads
    // when an image attachment or screen capture routes to the vision model.
    {
        let warmup_ollama = ollama.clone();
        let warmup_chat_model = default_model.clone();
        let warmup_embed_model = ollama_connector::DEFAULT_EMBED_MODEL.to_string();
        tokio::spawn(async move {
            // Both models load in parallel: sequential warmup leaves a cold-embed
            // window after the chat model finishes loading.
            let ollama_chat = warmup_ollama.clone();
            let ollama_embed = warmup_ollama.clone();
            let (r_chat, r_embed) = tokio::join!(
                ollama_chat.generate_raw(&warmup_chat_model, ".", 0.0),
                ollama_embed.embed(&warmup_embed_model, "warmup"),
            );
            if r_chat.is_ok() {
                tracing::info!(model = %warmup_chat_model, "warmup: chat model loaded");
            }
            if r_embed.is_ok() {
                tracing::info!(model = %warmup_embed_model, "warmup: embed model loaded");
            }
        });
    }

    // Automated mail sync: battery-aware interval poller
    // On AC power:      every 5 minutes
    // On battery power: no background polling — sync only on demand when user asks about mail
    if let Some(mail_for_poll) = mail.clone() {
        let db_poll = db.clone();
        let memory_poll = memory.clone();
        tokio::spawn(async move {
            // Initial sync on startup only when on AC (slight delay)
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            if is_on_ac_power() {
                match mail_sync_inner(db_poll.clone(), mail_for_poll.clone(), memory_poll.clone())
                    .await
                {
                    Ok((n, _)) => tracing::info!("mail auto-sync startup: {n} messages"),
                    Err(e) => tracing::warn!("mail auto-sync startup error: {e}"),
                }
            }
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300)); // 5 min
            interval.tick().await; // consume immediate tick
            loop {
                interval.tick().await;
                if !is_on_ac_power() {
                    tracing::debug!("mail auto-sync skipped: on battery");
                    continue;
                }
                match mail_sync_inner(db_poll.clone(), mail_for_poll.clone(), memory_poll.clone())
                    .await
                {
                    Ok((n, _)) if n > 0 => tracing::info!("mail auto-sync: {n} new messages"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("mail auto-sync error: {e}"),
                }
            }
        });
    }

    // FSEvents watcher: immediate sync when Apple Mail WAL changes
    if let Some(mail_for_fs) = mail.clone() {
        let db_fs = db.clone();
        let memory_fs = memory.clone();
        let home = dirs::home_dir().unwrap_or_default();
        let mail_wal = home.join("Library/Mail/V10/MailData/Envelope Index-wal");
        if mail_wal.exists() {
            // Bridge std mpsc → tokio mpsc so the receiver is Send
            let (tok_tx, mut tok_rx) = tokio::sync::mpsc::channel::<()>(4);
            use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
            let watcher_result = RecommendedWatcher::new(
                move |res: notify::Result<notify::Event>| {
                    if res.is_ok() {
                        let _ = tok_tx.try_send(());
                    }
                },
                Config::default(),
            );
            match watcher_result {
                Ok(mut watcher) => {
                    if watcher
                        .watch(&mail_wal, RecursiveMode::NonRecursive)
                        .is_ok()
                    {
                        tokio::spawn(async move {
                            let _watcher = watcher; // keep alive
                            loop {
                                if tok_rx.recv().await.is_none() {
                                    break;
                                }
                                // Debounce: drain any burst events
                                while tok_rx.try_recv().is_ok() {}
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                while tok_rx.try_recv().is_ok() {}
                                if is_on_ac_power() {
                                    tracing::info!("mail FSEvents: WAL changed, syncing");
                                    match mail_sync_inner(
                                        db_fs.clone(),
                                        mail_for_fs.clone(),
                                        memory_fs.clone(),
                                    )
                                    .await
                                    {
                                        Ok((n, _)) if n > 0 => {
                                            tracing::info!("mail FSEvents sync: {n} new")
                                        }
                                        Ok(_) => {}
                                        Err(e) => tracing::warn!("mail FSEvents sync error: {e}"),
                                    }
                                } else {
                                    tracing::debug!(
                                        "mail FSEvents: WAL changed, skipped (battery)"
                                    );
                                }
                            }
                        });
                    }
                }
                Err(e) => tracing::warn!("mail FSEvents watcher failed to init: {e}"),
            }
        }
    }

    // Rules engine — write default file if absent, then load + hot-reload
    let rules_path = data_dir.join("rules.yaml");
    if !rules_path.exists() {
        std::fs::write(&rules_path, DEFAULT_RULES_YAML)?;
    }
    let rules = Arc::new(RuleEngine::load_or_default(&rules_path));
    Arc::clone(&rules).spawn_hot_reload();

    let pending_approvals: Arc<std::sync::Mutex<HashMap<String, oneshot::Sender<bool>>>> =
        Arc::new(std::sync::Mutex::new(HashMap::new()));

    // Scan skills directories: repo skills/ first, then user skills dir (override by name).
    let skills = {
        let mut skills_dirs: Vec<std::path::PathBuf> = vec![];
        if let Ok(exe) = std::env::current_exe() {
            for candidate in [
                exe.parent().map(|p| p.join("skills")),
                exe.parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.parent())
                    .map(|p| p.join("skills")),
            ]
            .into_iter()
            .flatten()
            {
                if candidate.is_dir() {
                    skills_dirs.push(candidate);
                }
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            let cwd_skills = cwd.join("skills");
            if cwd_skills.is_dir() && !skills_dirs.contains(&cwd_skills) {
                skills_dirs.push(cwd_skills);
            }
        }
        skills_dirs.push(data_dir.join("skills")); // user override dir
        let loaded = bagent_skills::scan_dirs(&skills_dirs);
        tracing::info!(
            "skills: loaded {} — {:?}",
            loaded.len(),
            loaded
                .iter()
                .map(|s| s.manifest.name.as_str())
                .collect::<Vec<_>>()
        );
        Arc::new(loaded)
    };


    let task_rater = Arc::new(TaskRater::new());

    let codex = {
        let config = CodexConfig {
            binary_path: None, // auto-discover from $PATH
            timeout: std::time::Duration::from_secs(120),
        };
        match CodexConnector::new(config) {
            Ok(c) => {
                tracing::info!(
                    binary = %c.resolved_path().display(),
                    "Codex connector available"
                );
                Some(c)
            }
            Err(e) => {
                tracing::info!("Codex connector unavailable: {e}");
                None
            }
        }
    };

    // Odoo connector — starts unconfigured; Swift configures it lazily via POST /odoo/config.
    let odoo: Arc<RwLock<Option<OdooConnector>>> = Arc::new(RwLock::new(None));

    // WhatsApp connector — always present; autostarts only for prior paired sessions.
    let whatsapp = Arc::new(WhatsappConnector::new(WhatsappConfig::default()));
    if whatsapp.has_persisted_session() {
        let wa_autostart = whatsapp.clone();
        tokio::spawn(async move {
            tracing::info!("WhatsApp persisted session found; starting bridge");
            if let Err(e) = wa_autostart.start().await {
                tracing::warn!("WhatsApp autostart failed: {e}");
            }
        });
    }

    let state = AppState {
        db,
        db_path: data_dir.join("bagent.db"),
        token,
        default_model,
        debug_dir,
        classifier_model,
        vision_model,
        attachments_dir,
        ollama,
        mail,
        notes,
        fs,
        memory,
        prompt_builder,
        rules,
        pending_approvals,
        skills,
        task_rater,
        codex,
        odoo,
        whatsapp,
        runtime_refs: Arc::new(Mutex::new(HashMap::new())),
    };

    let shutdown_state = state.clone();
    let app = Router::new()
        .route("/health", get(health))
        .route("/models", get(models))
        .route("/chat", post(chat))
        .route("/embeddings", post(embeddings))
        .route("/approvals/pending", get(approvals_pending))
        .route("/approvals/:id/decide", post(approval_decide))
        .route("/rules", get(rules_get).post(rules_save))
        // Phase 4B — Sessions
        .route("/sessions", post(session_create).get(sessions_list))
        .route("/sessions/:id/turns", get(session_turns))
        .route("/sessions/:id", delete(session_delete))
        // Phase 4B — Memory
        .route("/memory", post(memory_insert).get(memory_list))
        .route("/memory/search", get(memory_search))
        .route("/memory/:id", delete(memory_delete))
        // Phase 5B — Attachments
        .route("/attachments", post(upload_attachment))
        .route("/attachments/:id", get(get_attachment))
        // Phase 4 — Mail
        .route("/mail/inbox", get(mail_inbox))
        .route("/mail/message/:rowid", get(mail_message))
        .route("/mail/sync", post(mail_sync))
        // Phase 5C — Mail attachments
        .route(
            "/mail/message/:rowid/attachments",
            get(mail_message_attachments),
        )
        .route(
            "/mail/message/:rowid/attachments/:idx",
            get(mail_message_attachment_bytes),
        )
        // Phase 5E — Open mail in Mail.app
        .route("/mail/open", post(mail_open))
        // Phase 4 — Notes
        .route("/notes/list", get(notes_list))
        .route("/notes/search", get(notes_search))
        .route("/notes/:pk", get(notes_get))
        // Phase 4G — Disk usage
        .route("/usage", get(disk_usage))
        .route("/mail/cache/clear", post(mail_cache_clear))
        // Phase 4H — Prompt trace debug
        .route("/debug/conversations/:id", get(debug_conversation))
        .route("/debug/traces/:id", get(debug_trace))
        // Skills
        .route("/skills", get(skills_list))
        .route("/skills/:name", get(skills_get))
        // Context plan debug
        // Phase 13A — Filesystem + app-open
        .route("/filesystem/roots", get(filesystem_roots))
        .route("/filesystem/search", post(filesystem_search))
        .route("/filesystem/read", post(filesystem_read))
        .route("/filesystem/metadata", get(filesystem_metadata))
        .route("/filesystem/reveal", post(filesystem_reveal))
        .route("/filesystem/open-folder", post(filesystem_open_folder))
        .route("/filesystem/open", post(filesystem_open))
        .route("/filesystem/open-with", post(filesystem_open_with))
        .route("/macos/open-app", post(macos_open_app))
        .route("/macos/focus-app", post(macos_focus_app))
        .route("/screen/intent", post(screen_intent_handler))
        // Phase 8 — Codex external-reasoning harness
        .route("/codex/status", get(codex_status_handler))
        .route("/codex/rate-task", post(codex_rate_task_handler))
        .route("/codex/run-task", post(codex_run_task_handler))
        // Phase 6 — Odoo connector
        .route("/odoo/config", post(odoo_config_handler))
        .route("/odoo/status", get(odoo_status_handler))
        .route("/odoo/open", post(odoo_open_handler))
        // Phase 11 — WhatsApp connector
        .route("/whatsapp/status", get(whatsapp_status_handler))
        .route("/whatsapp/start", post(whatsapp_start_handler))
        .route("/whatsapp/stop", post(whatsapp_stop_handler))
        .route("/whatsapp/qr", get(whatsapp_qr_handler))
        .route("/whatsapp/debug", get(whatsapp_debug_handler))
        .route("/whatsapp/logout", post(whatsapp_logout_handler))
        .route("/whatsapp/contacts", get(whatsapp_contacts_handler))
        .route("/whatsapp/chats", get(whatsapp_chats_handler))
        .route(
            "/whatsapp/chats/:id/messages",
            get(whatsapp_chat_messages_handler),
        )
        .route("/whatsapp/send", post(whatsapp_send_handler))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    std::fs::write(data_dir.join("daemon.port"), port.to_string())?;
    tracing::info!("bagentd listening on 127.0.0.1:{}", port);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown_state))
        .await?;
    let _ = std::fs::remove_file(data_dir.join("daemon.pid"));
    let _ = std::fs::remove_file(data_dir.join("daemon.port"));
    Ok(())
}

fn purge_legacy_context_data(data_dir: &std::path::Path, conn: &mut Connection) {
    let cleanup_sql = [
        "DELETE FROM memory_items",
        "DELETE FROM chat_turn_attachments",
        "DELETE FROM chat_turns",
        "DELETE FROM embeddings WHERE source IN ('memory_item','chat_turn')",
        "UPDATE sessions SET summary = NULL, metadata_json = NULL",
    ];
    for sql in cleanup_sql {
        if let Err(e) = conn.execute(sql, []) {
            tracing::debug!("legacy context purge skipped `{sql}`: {e}");
        }
    }

    let memories_dir = data_dir.join("memories");
    if memories_dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&memories_dir) {
            tracing::debug!("legacy memory mirror purge skipped: {e}");
        }
    }
}

// ── Filesystem handlers ───────────────────────────────────────────────────────

async fn filesystem_roots(State(state): State<AppState>) -> impl IntoResponse {
    let Some(fs) = state.fs else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Filesystem connector not accessible" })),
        );
    };
    let roots: Vec<String> = fs
        .policy
        .allowed_roots
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    (StatusCode::OK, Json(serde_json::json!({ "roots": roots })))
}

async fn filesystem_search(
    State(state): State<AppState>,
    Json(req): Json<FileSearchRequest>,
) -> impl IntoResponse {
    let Some(fs) = state.fs else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Filesystem connector not accessible" })),
        );
    };
    let policy = fs.policy.clone();
    match fs_search::search_files(policy, req).await {
        Ok(resp) => {
            audit_fs(
                &state.db,
                "filesystem_search",
                &serde_json::json!({
                    "result_count": resp.results.len(), "ok": true
                }),
            );
            (
                StatusCode::OK,
                Json(serde_json::to_value(resp).unwrap_or_default()),
            )
        }
        Err(e) => {
            audit_fs(
                &state.db,
                "filesystem_search",
                &serde_json::json!({ "ok": false, "error": e.to_string() }),
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn filesystem_read(
    State(state): State<AppState>,
    Json(req): Json<ReadTextRequest>,
) -> impl IntoResponse {
    let Some(fs) = state.fs else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Filesystem connector not accessible" })),
        );
    };
    let path_hash = sha256_str(&req.path);
    let policy = fs.policy.clone();
    match fs_search::read_text(policy, req).await {
        Ok(resp) => {
            audit_fs(
                &state.db,
                "filesystem_read_text",
                &serde_json::json!({
                    "path_hash": path_hash, "ok": true
                }),
            );
            (
                StatusCode::OK,
                Json(serde_json::to_value(resp).unwrap_or_default()),
            )
        }
        Err(e) => {
            audit_fs(
                &state.db,
                "filesystem_read_text",
                &serde_json::json!({ "path_hash": path_hash, "ok": false, "error": e.to_string() }),
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

#[derive(Deserialize)]
struct MetadataQuery {
    path: String,
}

async fn filesystem_metadata(
    State(state): State<AppState>,
    Query(q): Query<MetadataQuery>,
) -> impl IntoResponse {
    let Some(fs) = state.fs else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Filesystem connector not accessible" })),
        );
    };
    let policy = fs.policy.clone();
    match fs_search::metadata(policy, q.path).await {
        Ok(resp) => {
            audit_fs(
                &state.db,
                "filesystem_metadata",
                &serde_json::json!({ "ok": true }),
            );
            (
                StatusCode::OK,
                Json(serde_json::to_value(resp).unwrap_or_default()),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(Deserialize)]
struct PathBody {
    path: String,
}

#[derive(Deserialize)]
struct PathWithAppBody {
    path: String,
    app: String,
}

#[derive(Deserialize)]
struct AppBody {
    app: String,
}

async fn filesystem_reveal(
    State(state): State<AppState>,
    Json(body): Json<PathBody>,
) -> impl IntoResponse {
    let Some(fs) = state.fs else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Filesystem connector not accessible" })),
        );
    };
    let path_hash = sha256_str(&body.path);
    match state.rules.check("filesystem.reveal_in_finder", "{}") {
        ApprovalLevel::Forbidden => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": "blocked by rules" })),
            );
        }
        ApprovalLevel::Ask => {
            // REST route: ask is not supported (no SSE channel). Return 409.
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "approval required — use chat interface" })),
            );
        }
        ApprovalLevel::Auto => {}
    }
    match fs_open::reveal_in_finder(&fs.policy, &body.path).await {
        Ok(resp) => {
            audit_fs(
                &state.db,
                "filesystem_reveal_in_finder",
                &serde_json::json!({ "path_hash": path_hash, "ok": true }),
            );
            (
                StatusCode::OK,
                Json(serde_json::to_value(resp).unwrap_or_default()),
            )
        }
        Err(e) => {
            audit_fs(
                &state.db,
                "filesystem_reveal_in_finder",
                &serde_json::json!({ "path_hash": path_hash, "ok": false, "error": e.to_string() }),
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn filesystem_open_folder(
    State(state): State<AppState>,
    Json(body): Json<PathBody>,
) -> impl IntoResponse {
    let Some(fs) = state.fs else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Filesystem connector not accessible" })),
        );
    };
    let path_hash = sha256_str(&body.path);
    match state.rules.check("filesystem.open_folder", "{}") {
        ApprovalLevel::Forbidden => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": "blocked by rules" })),
            )
        }
        ApprovalLevel::Ask => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "approval required — use chat interface" })),
            )
        }
        ApprovalLevel::Auto => {}
    }
    match fs_open::open_folder(&fs.policy, &body.path).await {
        Ok(resp) => {
            audit_fs(
                &state.db,
                "filesystem_open_folder",
                &serde_json::json!({ "path_hash": path_hash, "ok": true }),
            );
            (
                StatusCode::OK,
                Json(serde_json::to_value(resp).unwrap_or_default()),
            )
        }
        Err(e) => {
            audit_fs(
                &state.db,
                "filesystem_open_folder",
                &serde_json::json!({ "path_hash": path_hash, "ok": false, "error": e.to_string() }),
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn filesystem_open(
    State(state): State<AppState>,
    Json(body): Json<PathBody>,
) -> impl IntoResponse {
    let Some(fs) = state.fs else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Filesystem connector not accessible" })),
        );
    };
    let path_hash = sha256_str(&body.path);
    match state.rules.check("filesystem.open_file", "{}") {
        ApprovalLevel::Forbidden => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": "blocked by rules" })),
            )
        }
        ApprovalLevel::Ask => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "approval required — use chat interface" })),
            )
        }
        ApprovalLevel::Auto => {}
    }
    match fs_open::open_file(&fs.policy, &body.path).await {
        Ok(resp) => {
            audit_fs(
                &state.db,
                "filesystem_open_file",
                &serde_json::json!({ "path_hash": path_hash, "ok": true }),
            );
            (
                StatusCode::OK,
                Json(serde_json::to_value(resp).unwrap_or_default()),
            )
        }
        Err(e) => {
            audit_fs(
                &state.db,
                "filesystem_open_file",
                &serde_json::json!({ "path_hash": path_hash, "ok": false, "error": e.to_string() }),
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn filesystem_open_with(
    State(state): State<AppState>,
    Json(body): Json<PathWithAppBody>,
) -> impl IntoResponse {
    let Some(fs) = state.fs else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Filesystem connector not accessible" })),
        );
    };
    let path_hash = sha256_str(&body.path);
    match state.rules.check("filesystem.open_file_with", "{}") {
        ApprovalLevel::Forbidden => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": "blocked by rules" })),
            )
        }
        ApprovalLevel::Ask => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "approval required — use chat interface" })),
            )
        }
        ApprovalLevel::Auto => {}
    }
    match fs_open::open_file_with(&fs.policy, &body.path, &body.app).await {
        Ok(resp) => {
            audit_fs(
                &state.db,
                "filesystem_open_file_with",
                &serde_json::json!({ "path_hash": path_hash, "app": body.app, "ok": true }),
            );
            (
                StatusCode::OK,
                Json(serde_json::to_value(resp).unwrap_or_default()),
            )
        }
        Err(e) => {
            audit_fs(
                &state.db,
                "filesystem_open_file_with",
                &serde_json::json!({ "path_hash": path_hash, "ok": false, "error": e.to_string() }),
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn macos_open_app(
    State(state): State<AppState>,
    Json(body): Json<AppBody>,
) -> impl IntoResponse {
    match state.rules.check("macos.open_app", "{}") {
        ApprovalLevel::Forbidden => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": "blocked by rules" })),
            )
        }
        ApprovalLevel::Ask => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "approval required — use chat interface" })),
            )
        }
        ApprovalLevel::Auto => {}
    }
    match fs_open::open_app(&body.app).await {
        Ok(resp) => {
            audit_fs(
                &state.db,
                "macos_open_app",
                &serde_json::json!({ "app": body.app, "ok": true }),
            );
            (
                StatusCode::OK,
                Json(serde_json::to_value(resp).unwrap_or_default()),
            )
        }
        Err(e) => {
            audit_fs(
                &state.db,
                "macos_open_app",
                &serde_json::json!({ "app": body.app, "ok": false, "error": e.to_string() }),
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn macos_focus_app(
    State(state): State<AppState>,
    Json(body): Json<AppBody>,
) -> impl IntoResponse {
    match state.rules.check("macos.focus_app", "{}") {
        ApprovalLevel::Forbidden => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": "blocked by rules" })),
            )
        }
        ApprovalLevel::Ask => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "approval required — use chat interface" })),
            )
        }
        ApprovalLevel::Auto => {}
    }
    match fs_open::focus_app(&body.app).await {
        Ok(resp) => {
            audit_fs(
                &state.db,
                "macos_focus_app",
                &serde_json::json!({ "app": body.app, "ok": true }),
            );
            (
                StatusCode::OK,
                Json(serde_json::to_value(resp).unwrap_or_default()),
            )
        }
        Err(e) => {
            audit_fs(
                &state.db,
                "macos_focus_app",
                &serde_json::json!({ "app": body.app, "ok": false, "error": e.to_string() }),
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

// ── Screen intent handler (Phase 7) ──────────────────────────────────────────

#[derive(Deserialize)]
struct ScreenIntentRequest {
    message: String,
}

/// POST /screen/intent — classifies whether the user turn requires screen context.
///
/// Uses `ContextPlanner` as the single source of truth: returns
/// `{ wants_screen, wants_ocr, wants_selection, task_type }` so the Swift
/// side can decide what to capture before sending the `/chat` request.
async fn screen_intent_handler(
    State(state): State<AppState>,
    Json(req): Json<ScreenIntentRequest>,
) -> impl IntoResponse {
    let classifier =
        ScreenIntentClassifier::new(state.ollama.clone(), state.classifier_model.clone());
    match classifier.classify(&req.message, "").await {
        Ok(intent) => (
            StatusCode::OK,
            Json(serde_json::to_value(&intent).unwrap_or_default()),
        ),
        Err(_) => {
            // Graceful degrade — caller treats unknown as "no screen needed"
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "action": "none",
                    "wants_screen": false,
                    "wants_ocr": false,
                    "wants_selection": false
                })),
            )
        }
    }
}

/// Helper: fire-and-forget audit entry for a filesystem/macos action.
fn audit_fs(db: &Arc<Mutex<Connection>>, action: &str, meta: &serde_json::Value) {
    if let Ok(db) = db.try_lock() {
        let _ = db.execute(
            "INSERT INTO audit_entries (action, payload, model) VALUES (?1, ?2, ?3)",
            rusqlite::params![action, meta.to_string(), ""],
        );
    }
}

fn sha256_str(s: &str) -> String {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bagent")
}

// ── Disk usage ────────────────────────────────────────────────────────────────

async fn disk_usage(State(state): State<AppState>) -> impl IntoResponse {
    let db_bytes = std::fs::metadata(&state.db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let attachments_bytes = dir_size(&state.attachments_dir);

    let (memory_items_count, chat_turns_count, mail_cache_count, embeddings_count): (
        i64,
        i64,
        i64,
        i64,
    ) = {
        let db = state.db.lock().await;
        let mc: i64 = db
            .query_row("SELECT COUNT(*) FROM memory_items", [], |r| r.get(0))
            .unwrap_or(0);
        let ct: i64 = db
            .query_row("SELECT COUNT(*) FROM chat_turns", [], |r| r.get(0))
            .unwrap_or(0);
        let mail: i64 = db
            .query_row("SELECT COUNT(*) FROM mail_cache", [], |r| r.get(0))
            .unwrap_or(0);
        let emb: i64 = db
            .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))
            .unwrap_or(0);
        (mc, ct, mail, emb)
    };

    let total_bytes = db_bytes + attachments_bytes;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "db_bytes": db_bytes,
            "attachments_bytes": attachments_bytes,
            "memory_items_count": memory_items_count,
            "chat_turns_count": chat_turns_count,
            "mail_cache_count": mail_cache_count,
            "embeddings_count": embeddings_count,
            "total_bytes": total_bytes
        })),
    )
}

async fn mail_cache_clear(State(state): State<AppState>) -> impl IntoResponse {
    let db = state.db.lock().await;
    let n = db
        .execute(
            "DELETE FROM mail_cache WHERE synced_at < strftime('%s', datetime('now', '-30 days'))",
            [],
        )
        .unwrap_or(0);
    (StatusCode::OK, Json(serde_json::json!({ "deleted": n })))
}

async fn debug_trace(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match find_prompt_debug_record(&state.debug_dir, |v| {
        v.get("prompt_trace_id").and_then(|x| x.as_str()) == Some(id.as_str())
    }) {
        Ok(Some(record)) => (StatusCode::OK, Json(record)),
        Ok(None) => {
            let matching_session_traces = read_prompt_debug_records(&state.debug_dir)
                .unwrap_or_default()
                .into_iter()
                .filter(|v| v.get("session_id").and_then(|x| x.as_str()) == Some(id.as_str()))
                .map(|v| {
                    serde_json::json!({
                        "prompt_trace_id": v.get("prompt_trace_id").cloned().unwrap_or_default(),
                        "created_at": v.get("created_at").cloned().unwrap_or_default(),
                        "user_message": v.get("user_message").cloned().unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>();
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "trace not found",
                    "hint": "This may be a conversation/session id. Use /debug/conversations/:id, or one of matching_prompt_traces with /debug/traces/:prompt_trace_id.",
                    "matching_prompt_traces": matching_session_traces,
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

// ── Skills handlers ───────────────────────────────────────────────────────────

async fn skills_list(State(state): State<AppState>) -> impl IntoResponse {
    let items: Vec<serde_json::Value> = state
        .skills
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.manifest.name,
                "description": s.manifest.description,
                "version": s.manifest.version,
                "risk": format!("{:?}", s.manifest.risk).to_lowercase(),
                "tags": s.manifest.tags,
                "allowed_tools": s.manifest.allowed_tools,
            })
        })
        .collect();
    (StatusCode::OK, Json(serde_json::json!({ "skills": items })))
}

async fn skills_get(State(state): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    match state.skills.iter().find(|s| s.manifest.name == name) {
        Some(skill) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "name": skill.manifest.name,
                "description": skill.manifest.description,
                "version": skill.manifest.version,
                "risk": format!("{:?}", skill.manifest.risk).to_lowercase(),
                "tags": skill.manifest.tags,
                "allowed_tools": skill.manifest.allowed_tools,
                "body": skill.body,
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("skill '{}' not found", name) })),
        ),
    }
}

// ── Debug: context plan ───────────────────────────────────────────────────────



async fn debug_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let (session, turns, stats) = {
        let db = state.db.lock().await;
        let session: Option<serde_json::Value> = db
            .query_row(
                "SELECT id, started_at, ended_at, language, summary, metadata_json \
                 FROM sessions WHERE id = ?1",
                rusqlite::params![id],
                |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, String>(0)?,
                        "started_at": r.get::<_, String>(1)?,
                        "ended_at": r.get::<_, Option<String>>(2)?,
                        "language": r.get::<_, Option<String>>(3)?,
                        "summary": r.get::<_, Option<String>>(4)?,
                        "metadata_json": r.get::<_, Option<String>>(5)?,
                    }))
                },
            )
            .ok();

        let turns: Vec<serde_json::Value> = db
            .prepare(
                "SELECT id, role, content, language, model, created_at FROM chat_turns \
                 WHERE session_id = ?1 ORDER BY created_at",
            )
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![id], |r| {
                    let content: String = r.get(2)?;
                    Ok(serde_json::json!({
                        "id": r.get::<_, String>(0)?,
                        "role": r.get::<_, String>(1)?,
                        "content_preview": preview_text(&content, 500),
                        "chars": content.len(),
                        "language": r.get::<_, String>(3)?,
                        "model": r.get::<_, Option<String>>(4)?,
                        "created_at": r.get::<_, String>(5)?,
                    }))
                })
                .ok()
                .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default();

        let stats = serde_json::json!({
            "memory_items_count": db.query_row("SELECT COUNT(*) FROM memory_items", [], |r| r.get::<_, i64>(0)).unwrap_or(0),
            "chat_turns_count": db.query_row("SELECT COUNT(*) FROM chat_turns WHERE session_id = ?1", rusqlite::params![id], |r| r.get::<_, i64>(0)).unwrap_or(0),
            "embeddings_count": db.query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get::<_, i64>(0)).unwrap_or(0),
            "mail_cache_count": db.query_row("SELECT COUNT(*) FROM mail_cache", [], |r| r.get::<_, i64>(0)).unwrap_or(0),
        });
        (session, turns, stats)
    };

    let traces = read_prompt_debug_records(&state.debug_dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|v| v.get("session_id").and_then(|x| x.as_str()) == Some(id.as_str()))
        .collect::<Vec<_>>();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "conversation_id": id,
            "session": session,
            "stats": stats,
            "turns": turns,
            "traces": traces,
        })),
    )
}

fn append_prompt_debug_record(
    debug_dir: &std::path::Path,
    record: &PromptDebugRecord,
) -> Result<()> {
    std::fs::create_dir_all(debug_dir)?;
    let path = debug_dir.join("prompt-traces.jsonl");
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 5 * 1024 * 1024 {
        let rotated = debug_dir.join("prompt-traces.1.jsonl");
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(&path, rotated);
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(record)?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn read_prompt_debug_records(debug_dir: &std::path::Path) -> Result<Vec<serde_json::Value>> {
    let path = debug_dir.join("prompt-traces.jsonl");
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    Ok(content
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect())
}

fn find_prompt_debug_record<F>(
    debug_dir: &std::path::Path,
    pred: F,
) -> Result<Option<serde_json::Value>>
where
    F: Fn(&serde_json::Value) -> bool,
{
    Ok(read_prompt_debug_records(debug_dir)?
        .into_iter()
        .rev()
        .find(pred))
}

fn debug_trace_preview(trace: &PromptTrace) -> String {
    let layers = trace
        .layers
        .iter()
        .filter(|l| l.included)
        .map(|l| l.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let recall = if trace.past_turn_candidates.is_empty() {
        "no past-chat candidates".to_string()
    } else {
        format!(
            "{} past-chat candidates not injected",
            trace.past_turn_candidates.len()
        )
    };
    let mail = trace
        .mail_search_trace
        .as_ref()
        .and_then(|v| v.get("attempts").and_then(|a| a.as_array()).map(Vec::len))
        .map(|n| format!("; mail search attempts={n}"))
        .unwrap_or_default();
    preview_text(&format!("{layers}; {recall}{mail}"), 180)
}





fn preview_text(s: &str, max: usize) -> String {
    let compact = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= max {
        compact
    } else {
        let end = compact.floor_char_boundary(max);
        format!("{}…", &compact[..end])
    }
}

fn redact_debug_text(s: &str) -> String {
    s.split_whitespace()
        .map(|part| {
            let lower = part.to_ascii_lowercase();
            if lower.starts_with("bearer")
                || lower.starts_with("sk-")
                || lower.contains("api_key")
                || lower.contains("authorization:")
            {
                "[REDACTED]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns true when the Mac is connected to AC power (not running on battery).
/// Uses `pmset -g batt` — fast, no extra deps. Falls back to true on error
/// so background tasks run as expected when power status is unknown.
fn is_on_ac_power() -> bool {
    let Ok(out) = std::process::Command::new("pmset")
        .args(["-g", "batt"])
        .output()
    else {
        return true;
    };
    let s = String::from_utf8_lossy(&out.stdout);
    // "Now drawing from 'AC Power'" or "'Battery Power'"
    s.contains("AC Power")
}

fn dir_size(path: &std::path::Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries.flatten().fold(0u64, |acc, entry| {
        let meta = entry.metadata().ok();
        if let Some(m) = meta {
            if m.is_dir() {
                acc + dir_size(&entry.path())
            } else {
                acc + m.len()
            }
        } else {
            acc
        }
    })
}

// ── Auth ──────────────────────────────────────────────────────────────────────

async fn require_auth(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: axum::extract::Request,
    next: middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let ok = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == format!("Bearer {}", state.token))
        .unwrap_or(false);
    if !ok {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}

// ── Core handlers ─────────────────────────────────────────────────────────────

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let odoo_configured = state.odoo.read().await.is_some();
    let wa_status = state
        .whatsapp
        .status()
        .await
        .unwrap_or(whatsapp_connector::WhatsappStatus {
            status: WhatsappConnectionStatus::Stopped,
            me: None,
            error: Some("status unavailable".into()),
            diagnostics: None,
        });
    Json(HealthResponse {
        status: "ok",
        ollama: state.ollama.is_up().await,
        model: state.default_model,
        classifier_model: state.classifier_model,
        connectors: ConnectorStatus {
            mail: state.mail.is_some(),
            notes: state.notes.is_some(),
            odoo: odoo_configured,
            whatsapp: WhatsappHealthStatus {
                connected: wa_status.status == WhatsappConnectionStatus::Ready,
                needs_qr: wa_status.status == WhatsappConnectionStatus::Qr,
                error: wa_status.error.clone(),
                status: wa_status.status.to_string(),
            },
        },
    })
}

async fn models(State(state): State<AppState>) -> impl IntoResponse {
    match state.ollama.models().await {
        Ok(names) => (StatusCode::OK, Json(serde_json::json!({ "models": names }))),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn approvals_pending(State(state): State<AppState>) -> impl IntoResponse {
    let db = state.db.lock().await;
    let items: Vec<serde_json::Value> = db
        .prepare(
            "SELECT id, tool_name, description, expires_at, created_at \
             FROM pending_approvals \
             WHERE decision IS NULL AND expires_at > datetime('now') \
             ORDER BY created_at",
        )
        .ok()
        .and_then(|mut s| {
            s.query_map([], |row| {
                Ok(serde_json::json!({
                    "id":          row.get::<_, String>(0)?,
                    "tool_name":   row.get::<_, String>(1)?,
                    "description": row.get::<_, Option<String>>(2)?,
                    "expires_at":  row.get::<_, String>(3)?,
                    "created_at":  row.get::<_, String>(4)?,
                }))
            })
            .ok()
            .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();
    Json(serde_json::json!({ "approvals": items }))
}

async fn approval_decide(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ApprovalDecideRequest>,
) -> impl IntoResponse {
    let sender = state.pending_approvals.lock().unwrap().remove(&id);
    if let Some(tx) = sender {
        let _ = tx.send(req.allow);
        let decision = if req.allow { "allow" } else { "deny" };
        let decided_at = chrono::Utc::now().to_rfc3339();
        if let Ok(db) = state.db.try_lock() {
            let _ = db.execute(
                "UPDATE pending_approvals SET decision = ?1, decided_at = ?2 WHERE id = ?3",
                rusqlite::params![decision, decided_at, id],
            );
            let _ = db.execute(
                "INSERT INTO audit_entries (action, payload, model) VALUES ('approval_decide', ?1, '')",
                rusqlite::params![serde_json::json!({"id": id, "decision": decision}).to_string()],
            );
        }
        (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "approval not found or already decided" })),
        )
    }
}

async fn rules_get(State(state): State<AppState>) -> impl IntoResponse {
    let yaml = state.rules.rules_yaml();
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        yaml,
    )
}

async fn rules_save(
    State(state): State<AppState>,
    Json(req): Json<RulesSaveRequest>,
) -> impl IntoResponse {
    match state.rules.save_yaml(&req.yaml) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn embeddings(
    State(state): State<AppState>,
    Json(req): Json<EmbedRequest>,
) -> impl IntoResponse {
    let model = req.model.as_deref().unwrap_or(DEFAULT_EMBED_MODEL);
    match state.ollama.embed(model, &req.input).await {
        Ok(vec) => (
            StatusCode::OK,
            Json(serde_json::json!({ "embedding": vec, "model": model })),
        ),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn chat(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Sse<ReceiverStream<Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel(64);
    let model = req.model.clone().unwrap_or(state.default_model.clone());
    let db = state.db.clone();
    let ollama = state.ollama.clone();
    let user_message = req.message.clone();
    let mail = state.mail.clone();
    let notes = state.notes.clone();
    let prompt_builder = state.prompt_builder.clone();
    let rules = state.rules.clone();
    let debug_dir = state.debug_dir.clone();
    let pending_approvals = state.pending_approvals.clone();
    let vision_model = state.vision_model.clone();
    let attachment_ids = req.attachment_ids.clone();
    // Screen context (Phase 7) — never persisted to disk
    let screen_image_b64 = req.screen_image_b64.clone();
    let screen_ocr_text = req.screen_ocr_text.clone();
    let active_app = req.active_app.clone();
    let selected_text = req.selected_text.clone();
    let source_mode = req.source_mode.clone();
    let skills = state.skills.clone();
    let task_rater = state.task_rater.clone();
    let fs_exec = state.fs.clone(); // kept for action execution in handler post-classify
    let runtime_refs = state.runtime_refs.clone();

    tokio::spawn(async move {
        let t0 = std::time::Instant::now();
        // Ensure session exists
        let session_id = match req.session_id.clone() {
            Some(id) => id,
            None => {
                let id = Uuid::new_v4().to_string();
                let now = chrono::Utc::now().to_rfc3339();
                if let Ok(db) = db.try_lock() {
                    let _ = db.execute(
                        "INSERT OR IGNORE INTO sessions (id, started_at) VALUES (?1, ?2)",
                        rusqlite::params![id, now],
                    );
                }
                id
            }
        };

        let history: Vec<Message> = Vec::new();
        let session_summary = None;
        let runtime_snapshot = load_runtime_refs(&runtime_refs, &session_id).await;
        let last_mail_ref = runtime_snapshot.mail;
        let last_file_ref = runtime_snapshot.file;
        let last_odoo_ref = runtime_snapshot.odoo;
        let last_whatsapp_ref = runtime_snapshot.whatsapp;
        tracing::info!(
            "chat timing: clean state loaded {}ms",
            t0.elapsed().as_millis()
        );

        // Detect language (simple heuristic: SK diacritics present?)
        let lang = if user_message.chars().any(|c| "áčďéíľĺňóôŕšťúýž".contains(c)) {
            "sk"
        } else {
            "en"
        };

        // ── Follow-up references ──────────────────────────────────────────────
        // The tool loop resolves "open it" / "the second one" from a short note
        // about the last things tools returned this session.
        let mut ref_notes: Vec<String> = Vec::new();
        if let Some(ref m) = last_mail_ref {
            ref_notes.push(format!(
                "Last mail seen: rowid={} subject=\"{}\" from {}",
                m.rowid, m.subject, m.sender
            ));
        }
        if let Some(ref f) = last_file_ref {
            ref_notes.push(format!("Last file seen: {}", f.path));
        }
        if let Some(ref o) = last_odoo_ref {
            ref_notes.push(format!(
                "Last Odoo record seen: model={} id={} name=\"{}\"",
                o.model, o.id, o.name
            ));
        }
        if let Some(ref w) = last_whatsapp_ref {
            ref_notes.push(format!(
                "Last WhatsApp chat seen: chat_id={} contact={}",
                w.chat_id,
                w.contact_name.as_deref().unwrap_or("?")
            ));
        }
        let tool_ctx: Option<String> = if ref_notes.is_empty() {
            None
        } else {
            Some(format!(
                "Recent tool references (for follow-ups like \"open it\"):\n{}",
                ref_notes.join("\n")
            ))
        };
        let _ = source_mode; // source hints are obsolete — the model picks tools itself


        // Load attachment records from DB and build context + image data for Ollama.
        struct AttachmentData {
            images_b64: Vec<String>,
            ctx: Option<String>,
            model_override: Option<String>,
            turn_ids: Vec<String>,
        }
        let att_data = {
            let mut images_b64: Vec<String> = Vec::new();
            let mut ctx_parts: Vec<String> = Vec::new();
            let mut has_image = false;
            let mut turn_ids: Vec<String> = Vec::new();

            if !attachment_ids.is_empty() {
                if let Ok(db_guard) = db.try_lock() {
                    for att_id in &attachment_ids {
                        let row: Result<(String, String, String, String, Option<String>), _> = db_guard.query_row(
                            "SELECT filename, kind, bytes_path, mime, extracted_text FROM attachments WHERE id = ?1",
                            rusqlite::params![att_id],
                            |r| Ok((
                                r.get::<_,String>(0)?,
                                r.get::<_,String>(1)?,
                                r.get::<_,String>(2)?,
                                r.get::<_,String>(3)?,
                                r.get::<_,Option<String>>(4)?,
                            )),
                        );
                        if let Ok((filename, kind, bytes_path, _mime, extracted_text)) = row {
                            turn_ids.push(att_id.clone());
                            if kind == "image" {
                                has_image = true;
                                if let Ok(bytes) = std::fs::read(&bytes_path) {
                                    images_b64.push(B64.encode(&bytes));
                                }
                                ctx_parts.push(format!(
                                    "### {} (obrázok — spracované modelom pre videnie)",
                                    filename
                                ));
                            } else {
                                let text = extracted_text
                                    .unwrap_or_else(|| "[obsah nedostupný]".to_string());
                                ctx_parts.push(format!("### {filename}\n{text}"));
                            }
                        }
                    }
                }
            }

            let ctx = if ctx_parts.is_empty() {
                None
            } else {
                Some(format!("Pripojené súbory:\n\n{}", ctx_parts.join("\n\n")))
            };

            let model_override = if has_image {
                // Auto-route image turns to the vision model even when the client
                // sends the selected chat model from Settings.
                if let Ok(db_guard) = db.try_lock() {
                    let _ = db_guard.execute(
                        "INSERT INTO audit_entries (action, payload, model) VALUES ('model_swap', ?1, ?2)",
                        rusqlite::params![
                            serde_json::json!({"from": model, "to": vision_model, "reason": "image_attachment"}).to_string(),
                            vision_model.clone()
                        ],
                    );
                }
                Some(vision_model.clone())
            } else {
                None
            };

            AttachmentData {
                images_b64,
                ctx,
                model_override,
                turn_ids,
            }
        };

        // ── Screen context injection (Phase 7) ────────────────────────────────
        // In-memory only — never written to the attachments table or disk.
        // Merges into the same att_data fields so existing vision-routing logic fires.
        let att_data = {
            let AttachmentData {
                mut images_b64,
                ctx,
                model_override,
                turn_ids,
            } = att_data;

            let mut screen_ctx_parts: Vec<String> = Vec::new();
            let mut has_screen_image = false;

            if let Some(b64) = &screen_image_b64 {
                images_b64.push(b64.clone());
                has_screen_image = true;
                screen_ctx_parts.push(
                    "### Snímka obrazovky (pii: true — zhrň obsah, necituj doslovne)".to_string(),
                );
            }
            if let Some(app) = &active_app {
                screen_ctx_parts.push(format!("### Aktívna aplikácia\n{app}"));
            }
            if let Some(sel) = &selected_text {
                if !sel.is_empty() {
                    screen_ctx_parts.push(format!("### Vybraný text (pii: true)\n{sel}"));
                }
            }
            if let Some(ocr) = &screen_ocr_text {
                if !ocr.is_empty() {
                    screen_ctx_parts.push(format!("### OCR text z obrazovky (pii: true)\n{ocr}"));
                }
            }

            let merged_ctx = match (ctx, screen_ctx_parts.is_empty()) {
                (Some(existing), false) => {
                    Some(format!("{existing}\n\n{}", screen_ctx_parts.join("\n\n")))
                }
                (None, false) => Some(screen_ctx_parts.join("\n\n")),
                (existing, true) => existing,
            };

            // Upgrade model_override to vision when a screen image was added but no
            // file-attachment already triggered the vision swap.
            let merged_override = if has_screen_image && model_override.is_none() {
                if let Ok(db_guard) = db.try_lock() {
                    let _ = db_guard.execute(
                        "INSERT INTO audit_entries (action, payload, model) VALUES ('model_swap', ?1, ?2)",
                        rusqlite::params![
                            serde_json::json!({"from": model, "to": vision_model, "reason": "screen_context"}).to_string(),
                            vision_model
                        ],
                    );
                }
                Some(vision_model.to_string())
            } else {
                model_override
            };

            AttachmentData {
                images_b64,
                ctx: merged_ctx,
                model_override: merged_override,
                turn_ids,
            }
        };

        let effective_model = att_data.model_override.clone().unwrap_or(model.clone());

        tracing::info!(
            "chat timing: tool_ctx fetched {}ms",
            t0.elapsed().as_millis()
        );

        // ── Task rating (Phase 8) ─────────────────────────────────────────────
        // Rate the task deterministically and emit a lightweight SSE event.
        // This does NOT route the task to Codex automatically — it is used for
        // UI hints and future Codex-offer flows.
        {
            let task_rating = task_rater.rate(&user_message, &[], None);
            if task_rating.codex_recommended {
                let _ = tx
                    .send(Ok(Event::default().data(
                        serde_json::json!({
                            "type":    "task_rating",
                            "level":   format!("{}", task_rating.level),
                            "score":   task_rating.score,
                            "reasons": task_rating.reasons,
                            "privacy_risk": format!("{}", task_rating.privacy_risk),
                        })
                        .to_string(),
                    )))
                    .await;
            }
        }

        // Select skills
        let selected_skills: Vec<SelectedSkill> = {
            let bagent_selected =
                skill_selector::select(&[], &skills, &user_message);
            bagent_selected
                .into_iter()
                .map(|s| SelectedSkill {
                    name: s.name,
                    body: s.body,
                })
                .collect()
        };

        let selected_memory = Vec::new();
        let corrections = Vec::new();
        let recall_candidates = Vec::new();

        tracing::info!(
            "chat timing: stateless context selected {}ms — {} cards, {} corrections",
            t0.elapsed().as_millis(),
            selected_memory.len(),
            corrections.len(),
        );

        // ── Build layered prompt ──────────────────────────────────────────────
        let prompt_trace_id = Uuid::new_v4().to_string();
        let mut prompt_trace: Option<PromptTrace> = None;
        let mut messages = match prompt_builder
            .build(
                &user_message,
                lang,
                &bagent_agent::ResponseLanguageHint::MatchUser,
                &selected_skills,
                &selected_memory,
                &corrections,
                tool_ctx,
                att_data.ctx,
                history,
                session_summary,
                recall_candidates,
                false,
                None,
                &user_message,
            )
            .await
        {
            Ok(mut built) => {
                if att_data.images_b64.is_empty() {
                    built.messages.push(Message::user(&user_message));
                } else {
                    built.messages.push(Message::user_with_images(
                        &user_message,
                        att_data.images_b64.clone(),
                    ));
                }
                built.trace.layers.push(bagent_agent::PromptLayerTrace {
                    name: "current_user_turn".to_string(),
                    role: "user".to_string(),
                    included: true,
                    chars: user_message.len(),
                    preview: preview_text(&user_message, 240),
                });
                let prompt_chars: usize = built.messages.iter().map(|m| m.content.len()).sum();
                let _ = tx
                    .send(Ok(Event::default().data(
                        serde_json::json!({
                            "type": "debug_trace",
                            "prompt_trace_id": &prompt_trace_id,
                            "session_id": &session_id,
                            "preview": debug_trace_preview(&built.trace),
                            "prompt_chars": prompt_chars,
                            "prompt_token_estimate": prompt_chars / 4,
                            "message_count": built.messages.len(),
                            "selected_skill_names": built.trace.selected_skill_names,
                            "selected_memory_ids": built.trace.selected_memory_ids,
                            "conversation_recall_injected": built.trace.conversation_recall_injected,
                        })
                        .to_string(),
                    )))
                    .await;
                prompt_trace = Some(built.trace);
                built.messages
            }
            Err(_) => {
                if att_data.images_b64.is_empty() {
                    vec![Message::user(&user_message)]
                } else {
                    vec![Message::user_with_images(
                        &user_message,
                        att_data.images_b64.clone(),
                    )]
                }
            }
        };

        // Stateless chat: do not persist user turns or attachment links.

        let prompt_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        tracing::info!(
            "chat timing: prompt built {}ms — {} msgs ~{} chars ~{} tokens",
            t0.elapsed().as_millis(),
            messages.len(),
            prompt_chars,
            prompt_chars / 4
        );
        let prompt_messages_for_log = messages.clone();

        // ── Agentic tool loop ─────────────────────────────────────────────────
        // The model drives all data access through tool calls and can only cite
        // what tools returned. Guardrails live in the dispatcher: rules engine
        // verdicts, PathPolicy (inside the fs connector), approval modal for
        // writes, per-turn budgets, and an audit entry per call.
        // Vision turns (image attachment / screen capture) skip tools — the
        // vision model answers directly from the injected context.
        use ollama_connector::{ChatStreamEvent, ToolCall as OllamaToolCall, ToolDef as OllamaToolDef};

        let mut tools: Vec<OllamaToolDef> = Vec::new();
        if att_data.model_override.is_none() {
            if mail.is_some() {
                tools.push(OllamaToolDef::function(
                    "mail_search",
                    "Search the user's Apple Mail. Returns message headers (rowid, subject, sender, date). \
                     Use mail_read with a rowid to get a message body. \
                     Put the sender's email address or name in `sender` when the user asks about mail from someone. \
                     IMPORTANT: Never describe a message that this tool did not return.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "sender": {"type": "string", "description": "Sender email address or name."},
                            "subject": {"type": "string", "description": "Subject substring."},
                            "keywords": {"type": "array", "items": {"type": "string"}, "description": "Terms that must ALL match sender or subject."},
                            "date_from": {"type": "string", "description": "ISO date YYYY-MM-DD."},
                            "date_to": {"type": "string", "description": "ISO date YYYY-MM-DD."},
                            "limit": {"type": "integer", "description": "Max results, default 10."}
                        }
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "mail_list_inbox",
                    "List the most recent inbox messages, optionally unread only.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "limit": {"type": "integer", "description": "Max results, default 10."},
                            "unread_only": {"type": "boolean"}
                        }
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "mail_read",
                    "Read the full body of a mail message by rowid (from mail_search / mail_list_inbox).",
                    serde_json::json!({
                        "type": "object",
                        "properties": {"rowid": {"type": "integer"}},
                        "required": ["rowid"]
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "mail_open",
                    "Open a mail message in the Mail app by rowid.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {"rowid": {"type": "integer"}},
                        "required": ["rowid"]
                    }),
                ));
            }
            if fs_exec.is_some() {
                tools.push(OllamaToolDef::function(
                    "filesystem_search_files",
                    "Search the user's Mac for files by name or content using macOS Spotlight. \
                     Use multiple Slovak/English synonym terms for best recall on Slovak documents. \
                     IMPORTANT: Never name or describe a file that was not returned by this tool.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "terms": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "Search terms (OR semantics). When the user's query is in English but the files are Slovak business documents, include Slovak synonyms and transliterations. E.g. 'customer statement' → ['zákazník','zakaznik','preplatk','saldokonto','výpis','prehľad']."
                            },
                            "roots": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "Folders to search, e.g. ['~/Downloads']. Omit to search all allowed folders."
                            },
                            "extensions": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "File extensions without dot, e.g. ['pdf','xlsx']."
                            },
                            "search_contents": {
                                "type": "boolean",
                                "description": "Also search inside document contents (needed when the filename doesn't match but contents do)."
                            },
                            "max_results": {
                                "type": "integer",
                                "description": "Max results to return. Default 10."
                            }
                        },
                        "required": ["terms"]
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "filesystem_read_text",
                    "Read the text content of a local file (PDF, Word, Excel, or plain text). \
                     Use this to inspect candidate files returned by filesystem_search_files.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {"type": "string", "description": "Absolute path to the file."}
                        },
                        "required": ["path"]
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "filesystem_open_file",
                    "Open a local file in its default application. Requires user approval.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {"type": "string", "description": "Absolute path to the file."}
                        },
                        "required": ["path"]
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "filesystem_open_file_with",
                    "Open a local file in a specific application. Requires user approval.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {"type": "string"},
                            "app": {"type": "string", "description": "App name, e.g. 'Microsoft Excel', 'Preview'."}
                        },
                        "required": ["path", "app"]
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "filesystem_reveal_in_finder",
                    "Reveal a local file in the macOS Finder.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {"type": "string"}
                        },
                        "required": ["path"]
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "macos_open_app",
                    "Launch or focus a macOS application by name.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "app": {"type": "string", "description": "App name, e.g. 'Mail', 'Finder', 'Preview'."}
                        },
                        "required": ["app"]
                    }),
                ));
            }
            if notes.is_some() {
                tools.push(OllamaToolDef::function(
                    "notes_search",
                    "Search Apple Notes by title/snippet. Returns note metadata; use notes_read for the body.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "query": {"type": "string"},
                            "limit": {"type": "integer", "description": "Max results, default 10."}
                        },
                        "required": ["query"]
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "notes_read",
                    "Read the body of an Apple Note by its coredata_id (from notes_search).",
                    serde_json::json!({
                        "type": "object",
                        "properties": {"coredata_id": {"type": "string"}},
                        "required": ["coredata_id"]
                    }),
                ));
            }
            tools.push(OllamaToolDef::function(
                "macos_switch_workspace",
                "Switch to an AeroSpace window-manager workspace by name/number.",
                serde_json::json!({
                    "type": "object",
                    "properties": {"workspace": {"type": "string"}},
                    "required": ["workspace"]
                }),
            ));
            // WhatsApp connector always exists; calls report politely when the bridge is down.
            tools.push(OllamaToolDef::function(
                "whatsapp_list_chats",
                "List the user's recent WhatsApp chats (chat_id, contact name, last message).",
                serde_json::json!({
                    "type": "object",
                    "properties": {"limit": {"type": "integer", "description": "Max chats, default 20."}}
                }),
            ));
            tools.push(OllamaToolDef::function(
                "whatsapp_chat_messages",
                "Read recent messages of one WhatsApp chat by chat_id (from whatsapp_list_chats).",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "chat_id": {"type": "string"},
                        "limit": {"type": "integer", "description": "Max messages, default 20."}
                    },
                    "required": ["chat_id"]
                }),
            ));
            tools.push(OllamaToolDef::function(
                "whatsapp_send_message",
                "Send ONE WhatsApp text message to a chat. Always requires explicit user approval.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "chat_id": {"type": "string"},
                        "message": {"type": "string"}
                    },
                    "required": ["chat_id", "message"]
                }),
            ));
            if state.odoo.read().await.is_some() {
                tools.push(OllamaToolDef::function(
                    "odoo_search_partners",
                    "Search Odoo partners (customers/suppliers) by name or email.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "query": {"type": "string"},
                            "limit": {"type": "integer", "description": "Max results, default 10."}
                        },
                        "required": ["query"]
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "odoo_my_invoices",
                    "List Odoo invoices. open_only=true returns only unpaid/partially-paid ones.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "open_only": {"type": "boolean"},
                            "limit": {"type": "integer", "description": "Max results, default 10."}
                        }
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "odoo_my_helpdesk_tickets",
                    "List the user's Odoo helpdesk tickets. open_only=true excludes closed stages.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "open_only": {"type": "boolean"},
                            "limit": {"type": "integer", "description": "Max results, default 10."}
                        }
                    }),
                ));
                tools.push(OllamaToolDef::function(
                    "odoo_get_record",
                    "Read a single Odoo record by model and id, e.g. model='res.partner', id=42.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "model": {"type": "string"},
                            "id": {"type": "integer"}
                        },
                        "required": ["model", "id"]
                    }),
                ));
            }
        }

        let mut full_response = String::new();

        if tools.is_empty() {
            // Vision turns / no connectors: single streamed answer, no tools.
            let token_stream = ollama.chat_stream(effective_model.clone(), messages.clone());
            tokio::pin!(token_stream);
            while let Some(result) = token_stream.next().await {
                match result {
                    Ok(token) => {
                        full_response.push_str(&token);
                        let ev = Event::default()
                            .data(serde_json::json!({"type":"token","content":token}).to_string());
                        if tx.send(Ok(ev)).await.is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Ok(err_event(&e.to_string()))).await;
                        return;
                    }
                }
            }
        } else {
            let mut found_file_ref: Option<FileRef> = None;
            let mut tool_calls_used: usize = 0;
            // ponytail: flat budgets — raise if real sessions hit them
            const MAX_ROUNDS: usize = 5;
            const MAX_TOOL_CALLS: usize = 8;

            'agent: for round in 0..=MAX_ROUNDS {
                // Final round or exhausted budget: no tools → model must answer.
                let round_tools = if round == MAX_ROUNDS || tool_calls_used >= MAX_TOOL_CALLS {
                    Vec::new()
                } else {
                    tools.clone()
                };
                let stream = ollama.chat_stream_with_tools(
                    effective_model.clone(),
                    messages.clone(),
                    round_tools,
                );
                tokio::pin!(stream);

                let mut round_text = String::new();
                let mut round_calls: Vec<OllamaToolCall> = Vec::new();
                while let Some(ev) = stream.next().await {
                    match ev {
                        Ok(ChatStreamEvent::Delta(token)) => {
                            round_text.push_str(&token);
                            let ev = Event::default().data(
                                serde_json::json!({"type":"token","content":token}).to_string(),
                            );
                            if tx.send(Ok(ev)).await.is_err() {
                                return;
                            }
                        }
                        Ok(ChatStreamEvent::ToolCalls(calls)) => round_calls.extend(calls),
                        Err(e) => {
                            let _ = tx.send(Ok(err_event(&e.to_string()))).await;
                            return;
                        }
                    }
                }

                if round_calls.is_empty() {
                    full_response = round_text;
                    break 'agent;
                }

                // Assistant turn carrying this round's calls (plus any preamble text).
                let mut assistant = Message::assistant(round_text);
                assistant.tool_calls = round_calls.clone();
                messages.push(assistant);

                for call in &round_calls {
                    tool_calls_used += 1;
                    let fn_name = &call.function.name;
                    let args = &call.function.arguments;
                    tracing::info!("tool loop call {}: {} {:?}", tool_calls_used, fn_name, args);
                    let _ = tx
                        .send(Ok(Event::default().data(
                            serde_json::json!({"type":"tool_call","tool": fn_name}).to_string(),
                        )))
                        .await;
                    audit_fs(&db, "tool_call", &serde_json::json!({"tool": fn_name}));

                    let tool_result: String = if tool_calls_used > MAX_TOOL_CALLS {
                        "Tool budget exhausted — answer now using what you have.".to_string()
                    } else {
                        match fn_name.as_str() {
                            // ── Mail ──────────────────────────────────────────
                            tool @ ("mail_search" | "mail_list_inbox" | "mail_read"
                            | "mail_open") => match (&mail, rules.check("mail_inbox", "{}")) {
                                (None, _) => {
                                    "Apple Mail connector unavailable (Full Disk Access not granted)."
                                        .to_string()
                                }
                                (_, ApprovalLevel::Forbidden) => {
                                    let _ = tx
                                        .send(Ok(Event::default().data(
                                            serde_json::json!({"type":"tool_blocked","tool":"mail_inbox"})
                                                .to_string(),
                                        )))
                                        .await;
                                    "Mail access blocked by rules.".to_string()
                                }
                                (Some(m), level) => {
                                    let approved = match level {
                                        ApprovalLevel::Ask => {
                                            request_tool_approval(
                                                &db,
                                                &pending_approvals,
                                                &tx,
                                                "mail_inbox",
                                                "Čítanie poštovej schránky (Apple Mail)",
                                            )
                                            .await
                                        }
                                        _ => true,
                                    };
                                    if !approved {
                                        "Mail access not approved by the user.".to_string()
                                    } else {
                                        match tool {
                                            "mail_search" => {
                                                let (result, mail_ref) =
                                                    tool_mail_search(m, args).await;
                                                if let Some(ref r) = mail_ref {
                                                    let _ = tx
                                                        .send(Ok(Event::default().data(
                                                            serde_json::json!({
                                                                "type": "mail_found",
                                                                "rowid": r.rowid,
                                                                "message_id": r.message_id,
                                                                "subject": r.subject,
                                                                "sender": r.sender,
                                                                "auto_open": false,
                                                            })
                                                            .to_string(),
                                                        )))
                                                        .await;
                                                    save_last_mail_ref(
                                                        &runtime_refs,
                                                        &session_id,
                                                        r,
                                                    )
                                                    .await;
                                                }
                                                result
                                            }
                                            "mail_list_inbox" => tool_mail_list_inbox(m, args).await,
                                            "mail_read" => {
                                                let (result, mail_ref) =
                                                    tool_mail_read(m, args).await;
                                                if let Some(ref r) = mail_ref {
                                                    save_last_mail_ref(
                                                        &runtime_refs,
                                                        &session_id,
                                                        r,
                                                    )
                                                    .await;
                                                }
                                                result
                                            }
                                            _ => tool_mail_open(m, args).await,
                                        }
                                    }
                                }
                            },

                            // ── Notes ─────────────────────────────────────────
                            tool @ ("notes_search" | "notes_read") => match &notes {
                                None => "Apple Notes connector unavailable.".to_string(),
                                Some(n) => match rules.check("notes_search", "{}") {
                                    ApprovalLevel::Forbidden => {
                                        "Notes access blocked by rules.".to_string()
                                    }
                                    _ => {
                                        if tool == "notes_search" {
                                            tool_notes_search(n, args).await
                                        } else {
                                            tool_notes_read(n, args).await
                                        }
                                    }
                                },
                            },

                            // ── WhatsApp ──────────────────────────────────────
                            "whatsapp_list_chats" => {
                                tool_whatsapp_list_chats(&state.whatsapp, args).await
                            }
                            "whatsapp_chat_messages" => {
                                let (result, wa_ref) =
                                    tool_whatsapp_chat_messages(&state.whatsapp, args).await;
                                if let Some(ref r) = wa_ref {
                                    save_last_whatsapp_ref(&runtime_refs, &session_id, r).await;
                                }
                                result
                            }
                            "whatsapp_send_message" => {
                                let chat_id =
                                    args["chat_id"].as_str().unwrap_or_default().to_string();
                                let text =
                                    args["message"].as_str().unwrap_or_default().to_string();
                                if chat_id.is_empty() || text.is_empty() {
                                    "chat_id and message are required.".to_string()
                                } else {
                                    let approved = request_tool_approval(
                                        &db,
                                        &pending_approvals,
                                        &tx,
                                        "whatsapp.send_message",
                                        &format!("WhatsApp → {chat_id}: {text}"),
                                    )
                                    .await;
                                    if !approved {
                                        "User did not approve sending the message.".to_string()
                                    } else {
                                        match state
                                            .whatsapp
                                            .send_message(
                                                WhatsappSendTarget::ChatId(chat_id),
                                                &text,
                                            )
                                            .await
                                        {
                                            Ok(_) => "Message sent.".to_string(),
                                            Err(e) => format!("WhatsApp send failed: {e}"),
                                        }
                                    }
                                }
                            }

                            // ── Odoo (read-only; writes are forbidden by rules) ─
                            tool @ ("odoo_search_partners" | "odoo_my_invoices"
                            | "odoo_my_helpdesk_tickets" | "odoo_get_record") => {
                                let guard = state.odoo.read().await;
                                match guard.as_ref() {
                                    None => "Odoo not connected — connect it in Settings first."
                                        .to_string(),
                                    Some(o) => {
                                        let (result, odoo_ref) = tool_odoo(o, tool, args).await;
                                        if let Some(ref r) = odoo_ref {
                                            let _ = tx
                                                .send(Ok(Event::default().data(
                                                    serde_json::json!({
                                                        "type": "odoo_found",
                                                        "model": r.model,
                                                        "record_id": r.id,
                                                        "name": r.name,
                                                        "url": r.url,
                                                    })
                                                    .to_string(),
                                                )))
                                                .await;
                                            save_last_odoo_ref(&runtime_refs, &session_id, r)
                                                .await;
                                        }
                                        result
                                    }
                                }
                            }

                            // ── Window management (AeroSpace) ─────────────────
                            "macos_switch_workspace" => {
                                match json_str_arg(args, "workspace") {
                                    None => "workspace is required.".to_string(),
                                    Some(ws) => match run_aerospace(&["workspace", &ws]).await {
                                        Ok(_) => format!("Switched to workspace {ws}."),
                                        Err(e) => format!("AeroSpace error: {e}"),
                                    },
                                }
                            }

                            // ── Filesystem ────────────────────────────────────
                            "filesystem_search_files" => match fs_exec.as_ref() {
                                None => "Filesystem connector unavailable.".to_string(),
                                Some(fs_c) => {
                                    let terms: Vec<String> = args["terms"]
                                        .as_array()
                                        .map(|a| {
                                            a.iter()
                                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                                .collect()
                                        })
                                        .unwrap_or_default();
                                    let query = terms.first().cloned().unwrap_or_default();
                                    let roots: Option<Vec<String>> =
                                        args["roots"].as_array().map(|a| {
                                            a.iter()
                                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                                .collect()
                                        });
                                    let extensions: Option<Vec<String>> =
                                        args["extensions"].as_array().map(|a| {
                                            a.iter()
                                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                                .collect()
                                        });
                                    let search_contents =
                                        args["search_contents"].as_bool().unwrap_or(false);
                                    let max_results = args["max_results"]
                                        .as_u64()
                                        .map(|n| n as usize)
                                        .unwrap_or(10);

                                    let req = FileSearchRequest {
                                        query,
                                        terms,
                                        roots,
                                        search_names: true,
                                        search_contents,
                                        extensions,
                                        include_hidden: false,
                                        max_results,
                                        max_depth: Some(8),
                                    };
                                    let policy = fs_c.policy.clone();
                                    match tokio::task::spawn_blocking(move || {
                                        fs_search::search_files_sync(&policy, req)
                                    })
                                    .await
                                    {
                                        Ok(Ok(resp)) => {
                                            audit_fs(
                                                &db,
                                                "filesystem_search",
                                                &serde_json::json!({
                                                    "result_count": resp.results.len(),
                                                    "ok": true
                                                }),
                                            );
                                            // Track top result for coreference
                                            if found_file_ref.is_none() {
                                                if let Some(top) = resp.results.first() {
                                                    found_file_ref = Some(FileRef {
                                                        path: top.path.clone(),
                                                        display_name: top.display_name.clone(),
                                                        kind: format!("{:?}", top.kind)
                                                            .to_lowercase(),
                                                    });
                                                }
                                            }
                                            serde_json::to_string(&resp)
                                                .unwrap_or_else(|_| "[]".to_string())
                                        }
                                        Ok(Err(e)) => {
                                            format!("{{\"error\":\"{}\"}}", e)
                                        }
                                        Err(e) => {
                                            format!("{{\"error\":\"{}\"}}", e)
                                        }
                                    }
                                }
                            },

                            "filesystem_read_text" => match fs_exec.as_ref() {
                                None => "Filesystem connector unavailable.".to_string(),
                                Some(fs_c) => {
                                    let path = args["path"].as_str().unwrap_or("").to_string();
                                    let req = ReadTextRequest {
                                        path,
                                        max_bytes: None,
                                        around_line: None,
                                    };
                                    let policy = fs_c.policy.clone();
                                    match tokio::task::spawn_blocking(move || {
                                        fs_search::read_text_sync(&policy, req)
                                    })
                                    .await
                                    {
                                        Ok(Ok(resp)) => {
                                            // Cap content to avoid huge context
                                            let content: String =
                                                resp.content.chars().take(4000).collect();
                                            let truncated_note = if resp.truncated {
                                                " [truncated]"
                                            } else {
                                                ""
                                            };
                                            format!(
                                                "[File: {}]\n{}{}",
                                                resp.path, content, truncated_note
                                            )
                                        }
                                        Ok(Err(e)) => format!("Error reading file: {e}"),
                                        Err(e) => format!("Error: {e}"),
                                    }
                                }
                            },

                            tool @ ("filesystem_open_file"
                            | "filesystem_open_file_with"
                            | "filesystem_reveal_in_finder"
                            | "filesystem_open_folder"
                            | "macos_open_app"
                            | "macos_focus_app") => {
                                // Derive the dotted rule name from the underscore tool name
                                let rule_name = match tool {
                                    "filesystem_open_file" => "filesystem.open_file",
                                    "filesystem_open_file_with" => "filesystem.open_file_with",
                                    "filesystem_reveal_in_finder" => "filesystem.reveal_in_finder",
                                    "filesystem_open_folder" => "filesystem.open_folder",
                                    "macos_open_app" => "macos.open_app",
                                    "macos_focus_app" => "macos.focus_app",
                                    _ => tool,
                                };
                                let path = args["path"].as_str().map(|s| s.to_string());
                                let app = args["app"].as_str().map(|s| s.to_string());
                                let approval_level = rules.check(rule_name, "{}");
                                let approved = match approval_level {
                                    ApprovalLevel::Auto => true,
                                    ApprovalLevel::Ask => {
                                        request_tool_approval(
                                            &db,
                                            &pending_approvals,
                                            &tx,
                                            rule_name,
                                            &format!(
                                                "Open: {}",
                                                path.as_deref().or(app.as_deref()).unwrap_or("?")
                                            ),
                                        )
                                        .await
                                    }
                                    ApprovalLevel::Forbidden => {
                                        let _ = tx
                                            .send(Ok(Event::default().data(
                                                serde_json::json!({
                                                    "type": "tool_blocked",
                                                    "tool": rule_name
                                                })
                                                .to_string(),
                                            )))
                                            .await;
                                        false
                                    }
                                };
                                if !approved {
                                    format!("Tool {rule_name} blocked — user did not approve.")
                                } else if rule_name.starts_with("macos.") {
                                    match app {
                                        Some(ref a) => match fs_open::open_app(a).await {
                                            Ok(_) => {
                                                audit_fs(
                                                    &db,
                                                    &rule_name.replace('.', "_"),
                                                    &serde_json::json!({"app": a, "ok": true}),
                                                );
                                                format!("Opened: {a}")
                                            }
                                            Err(e) => format!("Error: {e}"),
                                        },
                                        None => "Error: no app".to_string(),
                                    }
                                } else {
                                    match fs_exec.as_ref() {
                                        None => "Filesystem connector unavailable.".to_string(),
                                        Some(fs_c) => {
                                            let result: anyhow::Result<OpenResponse> =
                                                match rule_name {
                                                    "filesystem.open_file" => {
                                                        if let Some(ref p) = path {
                                                            fs_open::open_file(&fs_c.policy, p)
                                                                .await
                                                        } else {
                                                            Err(anyhow::anyhow!("no path"))
                                                        }
                                                    }
                                                    "filesystem.open_file_with" => {
                                                        if let (Some(ref p), Some(ref a)) =
                                                            (&path, &app)
                                                        {
                                                            fs_open::open_file_with(
                                                                &fs_c.policy,
                                                                p,
                                                                a,
                                                            )
                                                            .await
                                                        } else {
                                                            Err(anyhow::anyhow!("no path or app"))
                                                        }
                                                    }
                                                    "filesystem.reveal_in_finder" => {
                                                        if let Some(ref p) = path {
                                                            fs_open::reveal_in_finder(
                                                                &fs_c.policy,
                                                                p,
                                                            )
                                                            .await
                                                        } else {
                                                            Err(anyhow::anyhow!("no path"))
                                                        }
                                                    }
                                                    "filesystem.open_folder" => {
                                                        if let Some(ref p) = path {
                                                            fs_open::open_folder(&fs_c.policy, p)
                                                                .await
                                                        } else {
                                                            Err(anyhow::anyhow!("no path"))
                                                        }
                                                    }
                                                    _ => Err(anyhow::anyhow!("unknown")),
                                                };
                                            match result {
                                                Ok(ref resp) => {
                                                    let path_hash =
                                                        path.as_deref().map(sha256_str);
                                                    audit_fs(
                                                        &db,
                                                        &rule_name.replace('.', "_"),
                                                        &serde_json::json!({
                                                            "path_hash": path_hash,
                                                            "app": app,
                                                            "ok": true
                                                        }),
                                                    );
                                                    let _ = tx
                                                        .send(Ok(Event::default().data(
                                                            serde_json::json!({
                                                                "type": "file_opened",
                                                                "path": resp.path,
                                                                "app": resp.app,
                                                                "action": resp.action,
                                                            })
                                                            .to_string(),
                                                        )))
                                                        .await;
                                                    format!(
                                                        "Opened: {}",
                                                        path.as_deref()
                                                            .or(app.as_deref())
                                                            .unwrap_or("ok")
                                                    )
                                                }
                                                Err(ref e) => {
                                                    audit_fs(
                                                        &db,
                                                        &rule_name.replace('.', "_"),
                                                        &serde_json::json!({
                                                            "ok": false,
                                                            "error": e.to_string()
                                                        }),
                                                    );
                                                    format!("Error: {e}")
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            other => {
                                tracing::warn!("unknown tool: {}", other);
                                format!("Unknown tool: {other}. Answer with what you have or use a listed tool.")
                            }
                        }
                    };

                    messages.push(Message::tool_result(fn_name, tool_result));
                }
            } // end 'agent loop

            if let Some(ref fref) = found_file_ref {
                save_last_file_ref(&runtime_refs, &session_id, fref).await;
            }
        }

        let response_for_audit = full_response.clone();
        if let Ok(db) = db.try_lock() {
            let _ = db.execute(
                "INSERT INTO audit_entries (action, payload, model) VALUES (?1, ?2, ?3)",
                rusqlite::params!["chat", &user_message, &effective_model],
            );
        }

        if let Some(trace) = prompt_trace {
            let record = PromptDebugRecord {
                prompt_trace_id: prompt_trace_id.clone(),
                session_id: session_id.clone(),
                created_at: chrono::Utc::now().to_rfc3339(),
                user_message: redact_debug_text(&user_message),
                model: effective_model.clone(),
                language: lang.to_string(),
                prompt_chars,
                prompt_token_estimate: prompt_chars / 4,
                message_count: prompt_messages_for_log.len(),
                prompt_messages: prompt_messages_for_log
                    .iter()
                    .map(|m| PromptDebugMessage {
                        role: m.role.clone(),
                        content: redact_debug_text(&m.content),
                        images_count: m.images.len(),
                    })
                    .collect(),
                trace,
                response_preview: redact_debug_text(&preview_text(&response_for_audit, 600)),
                response_chars: response_for_audit.len(),
                elapsed_ms: t0.elapsed().as_millis(),
            };
            if let Err(e) = append_prompt_debug_record(&debug_dir, &record) {
                tracing::warn!("prompt debug log write failed: {e}");
            }
        }

        let _ = tx
            .send(Ok(Event::default().data(
                serde_json::json!({"type":"done","session_id": session_id}).to_string(),
            )))
            .await;
    });

    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

// ── Session handlers ──────────────────────────────────────────────────────────

async fn session_create(State(state): State<AppState>) -> impl IntoResponse {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let db = state.db.lock().await;
    match db.execute(
        "INSERT INTO sessions (id, started_at) VALUES (?1, ?2)",
        rusqlite::params![id, now],
    ) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "session_id": id })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn sessions_list(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "sessions": [] })))
}

async fn session_turns(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    (
        StatusCode::GONE,
        Json(serde_json::json!({ "error": "session history is disabled", "turns": [] })),
    )
}

async fn session_delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let db = state.db.lock().await;
    match db.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id]) {
        Ok(n) if n > 0 => (StatusCode::OK, Json(serde_json::json!({ "deleted": true }))),
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "session not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

// ── Memory handlers ───────────────────────────────────────────────────────────

async fn memory_insert(
    State(_state): State<AppState>,
    Json(_req): Json<MemoryInsertRequest>,
) -> impl IntoResponse {
    (
        StatusCode::GONE,
        Json(serde_json::json!({ "error": "memory is disabled" })),
    )
}

async fn memory_list(
    State(_state): State<AppState>,
    Query(_q): Query<MemorySearchQuery>,
) -> impl IntoResponse {
    (
        StatusCode::GONE,
        Json(serde_json::json!({ "error": "memory is disabled", "items": [] })),
    )
}

async fn memory_search(
    State(_state): State<AppState>,
    Query(_q): Query<MemorySearchQuery>,
) -> impl IntoResponse {
    (
        StatusCode::GONE,
        Json(serde_json::json!({ "error": "memory is disabled", "hits": [] })),
    )
}

async fn memory_delete(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    (
        StatusCode::GONE,
        Json(serde_json::json!({ "error": "memory is disabled" })),
    )
}

// ── Mail handlers ─────────────────────────────────────────────────────────────

async fn mail_inbox(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let Some(mail) = state.mail else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                serde_json::json!({ "error": "Mail connector not accessible. Grant Full Disk Access in System Settings → Privacy & Security." }),
            ),
        );
    };

    match tokio::task::spawn_blocking(move || mail.list_inbox(q.limit, q.unread)).await {
        Ok(Ok(msgs)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "messages": msgs })),
        ),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn mail_message(State(state): State<AppState>, Path(rowid): Path<i64>) -> impl IntoResponse {
    let Some(mail) = state.mail else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Mail connector not accessible." })),
        );
    };

    let mut msg = match tokio::task::spawn_blocking(move || mail.get_message(rowid)).await {
        Ok(Ok(Some(m))) => m,
        Ok(Ok(None)) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "message not found" })),
            )
        }
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    // emlx not locally cached → try AppleScript fallback (needs Automation → Mail)
    if msg.body.is_none() {
        if let Some(body) = apple_mail_connector::body_via_applescript(&msg.subject).await {
            msg.language = apple_mail_connector::detect_language(&body);
            msg.body = Some(body);
            msg.body_available = true;
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "message": msg, "pii": true })),
    )
}

// ── Phase 5E — Open mail in Mail.app ─────────────────────────────────────────

async fn mail_open(
    State(state): State<AppState>,
    Json(req): Json<MailOpenReq>,
) -> impl IntoResponse {
    // If we have a rowid but no message_id, try to resolve it from the emlx.
    let message_id: Option<String> = if req.message_id.is_some() {
        req.message_id.clone()
    } else if let (Some(rowid), Some(ref mc)) = (req.rowid, &state.mail) {
        let mc2 = mc.clone();
        tokio::task::spawn_blocking(move || mc2.get_message(rowid))
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten()
            .and_then(|m| m.message_id)
    } else {
        None
    };

    match apple_mail_connector::open_message(message_id.as_deref(), &req.subject, &req.sender).await
    {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "opened": true }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

// ── Phase 5C — Mail attachment handlers ──────────────────────────────────────

async fn mail_message_attachments(
    State(state): State<AppState>,
    Path(rowid): Path<i64>,
) -> impl IntoResponse {
    let Some(mail) = state.mail else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Mail connector not accessible." })),
        );
    };
    match tokio::task::spawn_blocking(move || mail.get_message(rowid)).await {
        Ok(Ok(Some(msg))) => (
            StatusCode::OK,
            Json(serde_json::json!({ "attachments": msg.attachments })),
        ),
        Ok(Ok(None)) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "message not found" })),
        ),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn mail_message_attachment_bytes(
    State(state): State<AppState>,
    Path((rowid, idx)): Path<(i64, usize)>,
) -> impl IntoResponse {
    let Some(mail) = state.mail else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Mail connector not accessible." })),
        );
    };
    match tokio::task::spawn_blocking(move || mail.get_message_attachment_base64(rowid, idx)).await
    {
        Ok(Ok((meta, b64))) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "filename": meta.filename,
                "mimetype": meta.mimetype,
                "size": meta.size,
                "data_base64": b64,
                "pii": true,
            })),
        ),
        Ok(Err(e)) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

// ── Phase 5B — Attachment upload + retrieval ──────────────────────────────────

const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024; // 20 MB

async fn upload_attachment(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    // Collect file bytes from multipart
    let mut filename = String::from("attachment");
    let mut mime = String::from("application/octet-stream");
    let mut file_bytes: Vec<u8> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        if let Some(name) = field.name() {
            if name != "file" {
                continue;
            }
        }
        if let Some(fn_) = field.file_name() {
            filename = fn_.to_string();
        }
        if let Some(ct) = field.content_type() {
            mime = ct.to_string();
        }

        match field.bytes().await {
            Ok(b) if b.len() <= MAX_ATTACHMENT_BYTES => {
                file_bytes = b.to_vec();
                break;
            }
            Ok(_) => {
                return (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Json(serde_json::json!({ "error": "Súbor je príliš veľký (max 20 MB)" })),
                );
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e.to_string() })),
                );
            }
        }
    }

    if file_bytes.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "no file field in multipart" })),
        );
    }

    // Compute SHA-256 for content-addressed storage
    let sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&file_bytes);
        format!("{:x}", hasher.finalize())
    };

    // Derive file extension from filename / MIME
    let ext = filename
        .rsplit('.')
        .next()
        .filter(|e| e.len() <= 6 && e.chars().all(|c| c.is_alphanumeric()))
        .unwrap_or("bin");
    let stored_name = format!("{sha256}.{ext}");
    let bytes_path = state.attachments_dir.join(&stored_name);

    // Write file (idempotent — same sha → same path, no overwrite needed)
    if !bytes_path.exists() {
        if let Err(e) = std::fs::write(&bytes_path, &file_bytes) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            );
        }
    }

    // Extract text / classify
    let ext_result = extract_attachment(&bytes_path, &mime);
    let (kind, extracted_text) = match ext_result {
        Ok(r) => (r.kind.as_str().to_string(), r.extracted_text),
        Err(_) => ("other".to_string(), None),
    };

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let size_bytes = file_bytes.len() as i64;

    let db = state.db.lock().await;
    // Dedup by sha256: reuse existing attachment id if already stored
    let existing_id: Option<String> = db
        .query_row(
            "SELECT id FROM attachments WHERE sha256 = ?1",
            rusqlite::params![sha256],
            |r| r.get(0),
        )
        .ok();

    let att_id = if let Some(eid) = existing_id {
        eid
    } else {
        let _ = db.execute(
            "INSERT INTO attachments (id, sha256, filename, mime, kind, bytes_path, extracted_text, size_bytes, created_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                id, sha256, filename, mime, kind,
                bytes_path.to_string_lossy().as_ref(),
                extracted_text.as_deref(),
                size_bytes,
                now,
            ],
        );
        id
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "attachment_id": att_id,
            "filename": filename,
            "mime": mime,
            "kind": kind,
            "size": size_bytes,
            "sha256": sha256,
        })),
    )
}

async fn get_attachment(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let db = state.db.lock().await;
    let row: Result<(String, String, String, i64, Option<String>), _> = db.query_row(
        "SELECT filename, mime, bytes_path, size_bytes, extracted_text FROM attachments WHERE id = ?1",
        rusqlite::params![id],
        |r| Ok((
            r.get::<_,String>(0)?,
            r.get::<_,String>(1)?,
            r.get::<_,String>(2)?,
            r.get::<_,i64>(3)?,
            r.get::<_,Option<String>>(4)?,
        )),
    );
    match row {
        Ok((filename, mime, bytes_path, size, extracted_text)) => {
            // Return base64-encoded bytes for images; metadata + text for others
            if mime.starts_with("image/") {
                if let Ok(bytes) = std::fs::read(&bytes_path) {
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "id": id,
                            "filename": filename,
                            "mime": mime,
                            "size": size,
                            "data_base64": B64.encode(&bytes),
                        })),
                    );
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": id,
                    "filename": filename,
                    "mime": mime,
                    "size": size,
                    "extracted_text": extracted_text,
                })),
            )
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "attachment not found" })),
        ),
    }
}

// ── Notes handlers ────────────────────────────────────────────────────────────

async fn notes_list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let Some(notes) = state.notes else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                serde_json::json!({ "error": "Notes connector not accessible. Grant Full Disk Access in System Settings → Privacy & Security." }),
            ),
        );
    };

    match tokio::task::spawn_blocking(move || notes.list_notes(q.limit)).await {
        Ok(Ok(items)) => (StatusCode::OK, Json(serde_json::json!({ "notes": items }))),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn notes_search(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    let Some(notes) = state.notes else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Notes connector not accessible." })),
        );
    };

    match tokio::task::spawn_blocking(move || notes.search_notes(&q.q, q.limit)).await {
        Ok(Ok(items)) => (StatusCode::OK, Json(serde_json::json!({ "notes": items }))),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn notes_get(State(state): State<AppState>, Path(pk): Path<i64>) -> impl IntoResponse {
    let Some(notes) = state.notes.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Notes connector not accessible." })),
        );
    };

    // Fetch metadata synchronously first
    let meta = match tokio::task::spawn_blocking({
        let notes = notes.clone();
        move || notes.get_note_metadata(pk)
    })
    .await
    {
        Ok(Ok(Some(n))) => n,
        Ok(Ok(None)) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "note not found" })),
            )
        }
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    if meta.is_locked {
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "note": meta, "pii": true })),
        );
    }

    // Fetch body via JXA
    let coredata_id = meta.coredata_id.clone();
    let body = notes.get_note_body(&coredata_id).await.ok().flatten();
    let lang = body
        .as_deref()
        .and_then(apple_notes_connector::detect_language);

    let mut note = meta;
    note.body = body;
    note.language = lang;

    (
        StatusCode::OK,
        Json(serde_json::json!({ "note": note, "pii": true })),
    )
}

// ── Approval helpers ─────────────────────────────────────────────────────────

/// Core approval logic: insert a `pending_approvals` DB row, register the
/// oneshot channel, optionally emit an SSE notification, then block until the
/// user decides (Allow/Deny) or the 60 s countdown elapses.
///
/// `sse_tx` — pass `Some(&tx)` from the chat SSE flow to emit the
/// `approval_requested` event; pass `None` for REST callers (the Swift app's
/// 1 s poll of `GET /approvals/pending` will surface the row automatically).
async fn request_approval_core(
    db: &Arc<Mutex<Connection>>,
    pending: &Arc<std::sync::Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    tool_name: &str,
    description: &str,
    sse_tx: Option<&mpsc::Sender<Result<Event, Infallible>>>,
) -> bool {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();

    if let Ok(db) = db.try_lock() {
        let _ = db.execute(
            "INSERT INTO pending_approvals (id, tool_name, description, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, tool_name, description, expires_at, now],
        );
    }

    let (send, recv) = oneshot::channel::<bool>();
    pending.lock().unwrap().insert(id.clone(), send);

    // Emit SSE event only when a chat stream is active.
    if let Some(tx) = sse_tx {
        let _ = tx
            .send(Ok(Event::default().data(
                serde_json::json!({
                    "type":        "approval_requested",
                    "id":          id,
                    "tool":        tool_name,
                    "description": description,
                    "expires_in":  60
                })
                .to_string(),
            )))
            .await;
    }

    match tokio::time::timeout(tokio::time::Duration::from_secs(60), recv).await {
        Ok(Ok(decision)) => {
            let decision_str = if decision { "allow" } else { "deny" };
            if let Ok(db) = db.try_lock() {
                let decided_at = chrono::Utc::now().to_rfc3339();
                let _ = db.execute(
                    "UPDATE pending_approvals SET decision=?1, decided_at=?2 WHERE id=?3",
                    rusqlite::params![decision_str, decided_at, id],
                );
                let _ = db.execute(
                    "INSERT INTO audit_entries (action, payload, model) VALUES ('approval', ?1, '')",
                    rusqlite::params![
                        serde_json::json!({"id": id, "tool": tool_name, "decision": decision_str})
                            .to_string()
                    ],
                );
            }
            decision
        }
        _ => {
            pending.lock().unwrap().remove(&id);
            if let Ok(db) = db.try_lock() {
                let now2 = chrono::Utc::now().to_rfc3339();
                let _ = db.execute(
                    "UPDATE pending_approvals SET decision='deny', decided_at=?1 WHERE id=?2",
                    rusqlite::params![now2, id],
                );
                let _ = db.execute(
                    "INSERT INTO audit_entries (action, payload, model) VALUES ('approval_timeout', ?1, '')",
                    rusqlite::params![
                        serde_json::json!({"id": id, "tool": tool_name}).to_string()
                    ],
                );
            }
            false
        }
    }
}

/// Convenience wrapper for the chat SSE path (always emits the SSE event).
async fn request_tool_approval(
    db: &Arc<Mutex<Connection>>,
    pending: &Arc<std::sync::Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    tx: &mpsc::Sender<Result<Event, Infallible>>,
    tool_name: &str,
    description: &str,
) -> bool {
    request_approval_core(db, pending, tool_name, description, Some(tx)).await
}

// ── Codex handlers (Phase 8) ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct CodexRateTaskRequest {
    description: String,
    #[serde(default)]
    context_sources: Vec<String>,
    #[serde(default)]
    privacy_hint: Option<String>,
}

#[derive(Deserialize)]
struct CodexRunTaskRequest {
    description: String,
    #[serde(default)]
    context_sources: Vec<String>,
    #[serde(default)]
    context_refs: Vec<String>,
    #[serde(default)]
    force_codex: bool,
}

/// `GET /codex/status`
async fn codex_status_handler(State(state): State<AppState>) -> impl IntoResponse {
    match &state.codex {
        Some(c) => {
            let version = c.version().await;
            Json(serde_json::json!({
                "available": true,
                "binary_path": c.resolved_path().to_string_lossy(),
                "version": version,
                "configured_path": null
            }))
        }
        None => Json(serde_json::json!({
            "available": false,
            "error": "codex_not_found"
        })),
    }
}

/// `POST /codex/rate-task`
async fn codex_rate_task_handler(
    State(state): State<AppState>,
    Json(req): Json<CodexRateTaskRequest>,
) -> impl IntoResponse {
    let rating = state.task_rater.rate(
        &req.description,
        &req.context_sources,
        req.privacy_hint.as_deref(),
    );
    Json(serde_json::json!({
        "level": format!("{}", rating.level),
        "score": rating.score,
        "codex_recommended": rating.codex_recommended,
        "requires_approval": rating.requires_approval,
        "privacy_risk": format!("{}", rating.privacy_risk),
        "suggested_context_scope": rating.suggested_context_scope,
        "reasons": rating.reasons,
    }))
}

/// `POST /codex/run-task`
async fn codex_run_task_handler(
    State(state): State<AppState>,
    Json(req): Json<CodexRunTaskRequest>,
) -> impl IntoResponse {
    use bagent_agent::TaskLevel;

    // 1. Rate the task.
    let rating = state
        .task_rater
        .rate(&req.description, &req.context_sources, None);

    // 2. Bail early if local model is sufficient and force_codex is not set.
    if matches!(
        rating.level,
        TaskLevel::LocalOnly | TaskLevel::LocalPreferred
    ) && !req.force_codex
    {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "ran": false,
                "reason": "local_sufficient",
                "rating": {
                    "level": format!("{}", rating.level),
                    "score": rating.score,
                    "reasons": rating.reasons,
                }
            })),
        );
    }

    // 3. Bail if Codex binary is unavailable.
    let connector = match &state.codex {
        Some(c) => c.clone(),
        None => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ran": false,
                    "error": "codex_not_found",
                    "message": "Codex CLI not found. Install it and configure the path in Settings."
                })),
            );
        }
    };

    // 4. Check the rules gate.
    match state.rules.check("codex.run_task", "{}") {
        ApprovalLevel::Forbidden => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "ran": false,
                    "error": "forbidden",
                    "message": "codex.run_task is forbidden by the rules engine."
                })),
            );
        }
        _ => {} // Auto or Ask — both proceed to explicit approval below.
    }

    // 5. Build a proposed context packet from context_refs (summaries only by default).
    let allowed_context: Vec<ContextItem> = req
        .context_refs
        .iter()
        .map(|r| {
            // Derive source from the ref prefix (e.g. "mail:rowid:123" → "mail")
            let source = r.split(':').next().unwrap_or("unknown").to_string();
            ContextItem {
                source,
                title: None,
                summary: format!("(Summary for {} pending user approval)", r),
                record_ref: Some(r.clone()),
                pii: true, // conservative default
            }
        })
        .collect();

    let context_packet = CodexContextPacket {
        user_request: req.description.clone(),
        allowed_context,
        expected_output: CodexExpectedOutput::Analysis,
        ..Default::default()
    };

    // 6. Build approval description text (shown in the modal).
    let sources_str = if req.context_sources.is_empty() {
        "none declared".to_string()
    } else {
        req.context_sources.join(", ")
    };
    let approval_description = format!(
        "Codex External Reasoning — {}\n\
         Level: {} | Privacy: {} | Sources: {}\n\
         Codex is an external harness. It will receive only the approved context packet \
         (summaries and record refs, no raw bodies). It cannot perform side effects directly.",
        req.description, rating.level, rating.privacy_risk, sources_str,
    );

    // 7. Request approval via the DB-backed poll path (no SSE channel needed).
    let approved = request_approval_core(
        &state.db,
        &state.pending_approvals,
        "codex.run_task",
        &approval_description,
        None, // REST path — Swift polls GET /approvals/pending
    )
    .await;

    // 8. Audit the attempt.
    let task_id = Uuid::new_v4().to_string();
    if !approved {
        audit_fs(
            &state.db,
            "codex_run_task",
            &serde_json::json!({
                "task_id": &task_id,
                "description": &req.description,
                "level": format!("{}", rating.level),
                "privacy_risk": format!("{}", rating.privacy_risk),
                "context_sources": &req.context_sources,
                "decision": "denied",
            }),
        );
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "ran": false,
                "reason": "denied"
            })),
        );
    }

    // 9. Run Codex.
    let codex_task = CodexTask {
        id: task_id.clone(),
        description: req.description.clone(),
        context_packet,
        task_level: format!("{}", rating.level),
        privacy_risk: format!("{}", rating.privacy_risk),
    };

    let run_result = match connector.run(&codex_task).await {
        Ok(r) => r,
        Err(e) => {
            audit_fs(
                &state.db,
                "codex_run_task",
                &serde_json::json!({
                    "task_id": &task_id,
                    "description": &req.description,
                    "level": format!("{}", rating.level),
                    "error": e.to_string(),
                }),
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "ran": false,
                    "error": "spawn_failed",
                    "message": e.to_string()
                })),
            );
        }
    };

    // 10. Audit the result (never include raw bodies).
    audit_fs(
        &state.db,
        "codex_run_task",
        &serde_json::json!({
            "task_id": &task_id,
            "description": &req.description,
            "level": format!("{}", rating.level),
            "privacy_risk": format!("{}", rating.privacy_risk),
            "context_sources": &req.context_sources,
            "exit_code": run_result.exit_code,
            "timed_out": run_result.timed_out,
            "output_hash": &run_result.output_hash,
        }),
    );

    // 11. Build structured response. Extract fields from parsed JSON output if
    //     available; fall back to plain text.
    let empty_vec: Vec<serde_json::Value> = vec![];
    let empty_str = serde_json::Value::String(String::new());
    let (summary, findings, conflicts, proposed_actions, drafts, questions) =
        if let Some(ref v) = run_result.parsed_output {
            (
                v.get("summary").cloned().unwrap_or(empty_str.clone()),
                v.get("findings")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                v.get("conflicts")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                v.get("proposed_actions")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                v.get("drafts")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                v.get("questions_for_user")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
            )
        } else {
            (
                serde_json::Value::String(run_result.result_text.clone()),
                empty_vec.clone(),
                empty_vec.clone(),
                empty_vec.clone(),
                empty_vec.clone(),
                empty_vec,
            )
        };

    // Truncate raw stdout/stderr for the response (they're already truncated to 64 KiB;
    // trim further for the API response).
    let stdout_snippet = if run_result.stdout.len() > 2048 {
        format!("{}…", &run_result.stdout[..2048])
    } else {
        run_result.stdout.clone()
    };
    let stderr_snippet = if run_result.stderr.len() > 1024 {
        format!("{}…", &run_result.stderr[..1024])
    } else {
        run_result.stderr.clone()
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ran": true,
            "task_id": task_id,
            "rating": {
                "level": format!("{}", rating.level),
                "score": rating.score,
                "privacy_risk": format!("{}", rating.privacy_risk),
                "reasons": rating.reasons,
            },
            "summary": summary,
            "findings": findings,
            "conflicts": conflicts,
            "proposed_actions": proposed_actions,
            "drafts": drafts,
            "questions_for_user": questions,
            "stdout_snippet": stdout_snippet,
            "stderr_snippet": stderr_snippet,
            "exit_code": run_result.exit_code,
            "timed_out": run_result.timed_out,
            "output_hash": run_result.output_hash,
        })),
    )
}

// ── Odoo handlers (Phase 6B) ─────────────────────────────────────────────────

/// Request body for `POST /odoo/config`.
#[derive(Deserialize)]
struct OdooConfigReq {
    base_url: String,
    db: String,
    username: String,
    api_key: String,
    /// Optional path to the `uvx` binary for non-standard installs.
    uvx_path: Option<String>,
}

/// `POST /odoo/config` — spawn MCP child, authenticate, and store the connector.
/// Doubles as the Settings "Test" button: returns version + uid + mcp_available on success.
async fn odoo_config_handler(
    State(state): State<AppState>,
    Json(req): Json<OdooConfigReq>,
) -> impl IntoResponse {
    let cfg = OdooConfig {
        base_url: req.base_url,
        db: req.db,
        username: req.username,
        api_key: req.api_key,
    };
    let uvx_override = req.uvx_path.as_deref();

    match OdooConnector::connect_with_uvx(cfg, uvx_override).await {
        Ok(conn) => {
            let version = conn.server_version.clone();
            let uid = conn.uid;
            let tool_count = conn.tool_count;
            *state.odoo.write().await = Some(conn);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "version": version,
                    "uid": uid,
                    "mcp_available": true,
                    "tool_count": tool_count,
                })),
            )
        }
        Err(OdooError::McpUnavailable(msg)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "ok": false,
                "mcp_available": false,
                "error": msg,
            })),
        ),
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "ok": false,
                "mcp_available": true,
                "error": e.to_string(),
            })),
        ),
    }
}

/// `GET /odoo/status` — current connector state (no network call).
async fn odoo_status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let guard = state.odoo.read().await;
    match &*guard {
        Some(conn) => Json(serde_json::json!({
            "configured": true,
            "connected": true,
            "mcp_available": true,
            "version": conn.server_version,
            "uid": conn.uid,
            "tool_count": conn.tool_count,
        })),
        None => Json(serde_json::json!({
            "configured": false,
            "connected": false,
            "mcp_available": false,
        })),
    }
}

#[derive(Deserialize)]
struct OdooOpenReq {
    url: String,
}

/// `POST /odoo/open` — open an Odoo record URL in Safari.
async fn odoo_open_handler(Json(body): Json<OdooOpenReq>) -> impl IntoResponse {
    match fs_open::open_url_in_safari(&body.url).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
        ),
    }
}

// ── WhatsApp handlers (Phase 11) ─────────────────────────────────────────────

/// `GET /whatsapp/status`
async fn whatsapp_status_handler(State(state): State<AppState>) -> impl IntoResponse {
    match state.whatsapp.status().await {
        Ok(s) => {
            let me_name =
                s.me.as_ref()
                    .and_then(|me| me.name.clone().or_else(|| me.push_name.clone()));
            let me_phone = s.me.as_ref().and_then(|me| me.number.clone());
            let last_loading = s
                .diagnostics
                .as_ref()
                .and_then(|d| d.get("last_loading"))
                .cloned();
            let last_state = s
                .diagnostics
                .as_ref()
                .and_then(|d| d.get("last_state"))
                .cloned();
            Json(serde_json::json!({
                "status": s.status.to_string(),
                "connected": s.status == WhatsappConnectionStatus::Ready,
                "needs_qr": s.status == WhatsappConnectionStatus::Qr,
                "me": s.me,
                "me_name": me_name,
                "me_phone": me_phone,
                "error": s.error,
                "last_loading": last_loading,
                "last_state": last_state,
                "diagnostics": s.diagnostics,
            }))
        }
        Err(e) => Json(serde_json::json!({
            "status": "error",
            "connected": false,
            "needs_qr": false,
            "error": e.to_string(),
        })),
    }
}

/// `POST /whatsapp/start`
async fn whatsapp_start_handler(State(state): State<AppState>) -> impl IntoResponse {
    match state.whatsapp.start().await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
        ),
    }
}

/// `POST /whatsapp/stop`
async fn whatsapp_stop_handler(State(state): State<AppState>) -> impl IntoResponse {
    match state.whatsapp.stop().await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
        ),
    }
}

/// `GET /whatsapp/qr`
async fn whatsapp_qr_handler(State(state): State<AppState>) -> impl IntoResponse {
    match state.whatsapp.qr().await {
        Ok(qr) => (StatusCode::OK, Json(serde_json::json!({ "qr": qr }))),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "qr": null, "error": e.to_string() })),
        ),
    }
}

/// `GET /whatsapp/debug`
async fn whatsapp_debug_handler(State(state): State<AppState>) -> impl IntoResponse {
    match state.whatsapp.debug().await {
        Ok(debug) => (StatusCode::OK, Json(debug)),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// `POST /whatsapp/logout`
async fn whatsapp_logout_handler(State(state): State<AppState>) -> impl IntoResponse {
    let _ = state.whatsapp.logout().await;
    let _ = state.whatsapp.stop().await;
    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct WhatsappContactsQuery {
    limit: Option<usize>,
}

/// `GET /whatsapp/contacts?limit=N`
async fn whatsapp_contacts_handler(
    State(state): State<AppState>,
    Query(q): Query<WhatsappContactsQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    match state.whatsapp.list_contacts(limit).await {
        Ok(contacts) => (
            StatusCode::OK,
            Json(serde_json::to_value(contacts).unwrap_or_default()),
        ),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(Deserialize)]
struct WhatsappChatsQuery {
    limit: Option<usize>,
}

/// `GET /whatsapp/chats?limit=N`
async fn whatsapp_chats_handler(
    State(state): State<AppState>,
    Query(q): Query<WhatsappChatsQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(30).min(200);
    match state.whatsapp.list_chats(limit).await {
        Ok(chats) => (
            StatusCode::OK,
            Json(serde_json::to_value(chats).unwrap_or_default()),
        ),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(Deserialize)]
struct WhatsappMessagesQuery {
    limit: Option<usize>,
    before: Option<i64>,
}

/// `GET /whatsapp/chats/:id/messages?limit=N&before=TS`
async fn whatsapp_chat_messages_handler(
    State(state): State<AppState>,
    Path(chat_id): Path<String>,
    Query(q): Query<WhatsappMessagesQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(20).min(100);
    match state
        .whatsapp
        .chat_messages(&chat_id, limit, q.before)
        .await
    {
        Ok(msgs) => (
            StatusCode::OK,
            Json(serde_json::to_value(msgs).unwrap_or_default()),
        ),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(Deserialize)]
struct WhatsappSendReq {
    /// WhatsApp chat JID (mutually exclusive with `phone`).
    chat_id: Option<String>,
    /// Phone number in any format (mutually exclusive with `chat_id`).
    phone: Option<String>,
    /// Exact message text. Required.
    text: String,
}

/// `POST /whatsapp/send`
///
/// # Approval contract (trap #1 from plan)
///
/// The enforcement floor lives **here**, not in `rules.yaml`.
/// We call `request_approval_core` regardless of the `rules.check()` result,
/// unless the rule is `Forbidden` (which blocks immediately).
/// This ensures the invariant holds even for existing installations that have
/// an old `rules.yaml` on disk which does not contain the new WhatsApp rule.
async fn whatsapp_send_handler(
    State(state): State<AppState>,
    Json(req): Json<WhatsappSendReq>,
) -> impl IntoResponse {
    // Basic validation
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "sent": false, "reason": "text_empty" })),
        );
    }
    let target = match (req.chat_id.clone(), req.phone.clone()) {
        (Some(id), _) => WhatsappSendTarget::ChatId(id),
        (None, Some(ph)) => WhatsappSendTarget::Phone(ph),
        (None, None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "sent": false, "reason": "chat_id_or_phone_required" })),
            );
        }
    };
    let recipient_display = req
        .chat_id
        .as_deref()
        .or(req.phone.as_deref())
        .unwrap_or("unknown");

    // Rules gate — Forbidden blocks immediately; Auto and Ask both proceed to approval.
    match state.rules.check("whatsapp.send_message", "{}") {
        ApprovalLevel::Forbidden => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "sent": false,
                    "reason": "forbidden_by_rules"
                })),
            );
        }
        _ => {} // Auto or Ask — BOTH proceed to explicit approval below (trap #1).
    }

    let text_preview = if text.len() > 60 {
        format!("{}… ({} znakov)", &text[..60], text.len())
    } else {
        text.clone()
    };
    let audit_description = format!(
        "Odoslať WhatsApp správu — Príjemca: {} | Náhľad: {}",
        recipient_display, text_preview
    );

    // Request approval (REST path — Swift polls GET /approvals/pending).
    // Note: approval modal shows `approval_description` with full text;
    //       audit row stores `audit_description` (truncated preview, no full body).
    let approved = request_approval_core(
        &state.db,
        &state.pending_approvals,
        "whatsapp.send_message",
        &audit_description, // stored in audit_entries — redacted (trap #2)
        None,               // REST path; no SSE channel
    )
    .await;

    if !approved {
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "sent": false, "reason": "denied" })),
        );
    }

    match state.whatsapp.send_message(target, &text).await {
        Ok(msg_ref) => {
            tracing::info!(
                message_id = %msg_ref.message_id,
                "WhatsApp message sent"
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "sent": true,
                    "message_id": msg_ref.message_id,
                    "chat_id": msg_ref.chat_id,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("WhatsApp send error: {e}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "sent": false,
                    "reason": "send_error",
                    "error": e.to_string(),
                })),
            )
        }
    }
}

// ── Mail sync (incremental) ───────────────────────────────────────────────────

/// Core sync logic shared by the HTTP handler, interval poller, and FSEvents watcher.
async fn mail_sync_inner(
    db: Arc<Mutex<Connection>>,
    mail: MailConnector,
    _memory: Arc<MemoryStore>,
) -> Result<(usize, i64), String> {
    // Determine last sync and whether this is a first sync (deeper history)
    let (last_sync, is_first): (i64, bool) = {
        let db_lock = db.lock().await;
        let result: rusqlite::Result<i64> = db_lock.query_row(
            "SELECT last_sync_at FROM connectors WHERE kind = 'apple_mail'",
            [],
            |r| r.get(0),
        );
        match result {
            Ok(ts) => (ts, false),
            Err(_) => (0, true),
        }
    };

    let fetch_limit: usize = if is_first { 5000 } else { 500 };

    let new_msgs = tokio::task::spawn_blocking(move || mail.list_since(last_sync, fetch_limit))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    let count = new_msgs.len();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    {
        let db_lock = db.lock().await;
        for msg in &new_msgs {
            db_lock.execute(
                "INSERT OR REPLACE INTO mail_cache
                 (rowid, subject, sender, sender_display, received_at, is_read, mailbox_url, language, synced_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                rusqlite::params![
                    msg.rowid, &msg.subject, &msg.sender, &msg.sender_display,
                    msg.received_at, msg.is_read as i64, &msg.mailbox_url,
                    &msg.language, now
                ],
            ).ok();
        }
        db_lock
            .execute(
                "INSERT INTO connectors (kind, config_json, enabled, last_sync_at)
             VALUES ('apple_mail','{}',1,?1)
             ON CONFLICT(kind) DO UPDATE SET last_sync_at = ?1",
                rusqlite::params![now],
            )
            .ok();
    }

    Ok((count, now))
}

async fn mail_sync(State(state): State<AppState>) -> impl IntoResponse {
    let Some(mail) = state.mail.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Mail connector not accessible." })),
        );
    };

    match mail_sync_inner(state.db.clone(), mail, state.memory.clone()).await {
        Ok((count, now)) => {
            let total: i64 = {
                let db = state.db.lock().await;
                db.query_row("SELECT COUNT(*) FROM mail_cache", [], |r| r.get(0))
                    .unwrap_or(0)
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "synced": count,
                    "total_cached": total,
                    "last_sync_at": now
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        ),
    }
}

// ── Tool context injection ────────────────────────────────────────────────────























// ── WhatsApp DB helpers ───────────────────────────────────────────────────────




// ── AeroSpace executor ────────────────────────────────────────────────────────

/// Resolve the `aerospace` binary path: try $PATH first, then the bundled
/// in-app binary. Returns `None` if AeroSpace is not installed.
async fn find_aerospace_binary() -> Option<std::path::PathBuf> {
    // Try $PATH via `which`
    if let Ok(out) = tokio::process::Command::new("which")
        .arg("aerospace")
        .output()
        .await
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(std::path::PathBuf::from(s));
            }
        }
    }
    // Bundled fallback
    let bundled =
        std::path::PathBuf::from("/Applications/AeroSpace.app/Contents/Resources/aerospace");
    if bundled.exists() {
        Some(bundled)
    } else {
        None
    }
}

/// Run an `aerospace` subcommand. Returns `Ok(stdout)` on success,
/// `Err` on binary-not-found or non-zero exit (caller logs and silently degrades).
async fn run_aerospace(args: &[&str]) -> anyhow::Result<String> {
    let bin = find_aerospace_binary()
        .await
        .ok_or_else(|| anyhow::anyhow!("aerospace binary not found"))?;
    let out = tokio::process::Command::new(&bin)
        .args(args)
        .output()
        .await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("aerospace {:?} failed: {}", args, stderr.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}




// ── Context management ────────────────────────────────────────────────────────

async fn load_runtime_refs(
    refs: &Arc<Mutex<HashMap<String, RuntimeRefs>>>,
    session_id: &str,
) -> RuntimeRefs {
    refs.lock()
        .await
        .get(session_id)
        .cloned()
        .unwrap_or_default()
}

async fn save_last_mail_ref(
    refs: &Arc<Mutex<HashMap<String, RuntimeRefs>>>,
    session_id: &str,
    mail_ref: &MailRef,
) {
    refs.lock()
        .await
        .entry(session_id.to_string())
        .or_default()
        .mail = Some(mail_ref.clone());
}

async fn save_last_file_ref(
    refs: &Arc<Mutex<HashMap<String, RuntimeRefs>>>,
    session_id: &str,
    file_ref: &FileRef,
) {
    refs.lock()
        .await
        .entry(session_id.to_string())
        .or_default()
        .file = Some(file_ref.clone());
}

async fn save_last_odoo_ref(
    refs: &Arc<Mutex<HashMap<String, RuntimeRefs>>>,
    session_id: &str,
    odoo_ref: &OdooRecordRef,
) {
    refs.lock()
        .await
        .entry(session_id.to_string())
        .or_default()
        .odoo = Some(odoo_ref.clone());
}

async fn save_last_whatsapp_ref(
    refs: &Arc<Mutex<HashMap<String, RuntimeRefs>>>,
    session_id: &str,
    whatsapp_ref: &WhatsappRef,
) {
    refs.lock()
        .await
        .entry(session_id.to_string())
        .or_default()
        .whatsapp = Some(whatsapp_ref.clone());
}

fn err_event(msg: &str) -> Event {
    Event::default().data(serde_json::json!({"type":"error","message":msg}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purge_legacy_context_data_clears_memory_chat_and_mirror() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE memory_items (id TEXT);
            CREATE TABLE chat_turns (id TEXT);
            CREATE TABLE chat_turn_attachments (chat_turn_id TEXT, attachment_id TEXT);
            CREATE TABLE embeddings (item_id TEXT, source TEXT);
            CREATE TABLE sessions (id TEXT, summary TEXT, metadata_json TEXT);
            INSERT INTO memory_items VALUES ('m1');
            INSERT INTO chat_turns VALUES ('t1');
            INSERT INTO chat_turn_attachments VALUES ('t1', 'a1');
            INSERT INTO embeddings VALUES ('m1', 'memory_item');
            INSERT INTO embeddings VALUES ('t1', 'chat_turn');
            INSERT INTO embeddings VALUES ('wa1', 'whatsapp');
            INSERT INTO sessions VALUES ('s1', 'old summary', '{\"last_mail_ref\":{}}');
            ",
        )
        .unwrap();

        let data_dir = std::env::temp_dir().join(format!("bagent-purge-test-{}", Uuid::new_v4()));
        let memories_dir = data_dir.join("memories");
        std::fs::create_dir_all(&memories_dir).unwrap();
        std::fs::write(memories_dir.join("old.md"), "old memory").unwrap();

        purge_legacy_context_data(&data_dir, &mut conn);

        for table in ["memory_items", "chat_turns", "chat_turn_attachments"] {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "{table} should be empty");
        }
        let embeddings: i64 = conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(embeddings, 1, "non memory/chat embeddings must remain");
        let cleared: (Option<String>, Option<String>) = conn
            .query_row("SELECT summary, metadata_json FROM sessions", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(cleared, (None, None));
        assert!(!memories_dir.exists());

        let _ = std::fs::remove_dir_all(data_dir);
    }











}

// ── Tool-loop dispatch helpers ────────────────────────────────────────────────
// Thin wrappers over the connectors; the loop in `chat` gates them via the
// rules engine / approvals before calling.

fn json_str_arg(args: &serde_json::Value, key: &str) -> Option<String> {
    args[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn json_day_ts(args: &serde_json::Value, key: &str, end_of_day: bool) -> Option<i64> {
    let d = chrono::NaiveDate::parse_from_str(args[key].as_str()?, "%Y-%m-%d").ok()?;
    let t = if end_of_day {
        d.and_hms_opt(23, 59, 59)?
    } else {
        d.and_hms_opt(0, 0, 0)?
    };
    Some(t.and_utc().timestamp())
}

fn mail_headers_json(msgs: &[apple_mail_connector::MailMessage]) -> String {
    let items: Vec<serde_json::Value> = msgs
        .iter()
        .map(|m| {
            serde_json::json!({
                "rowid": m.rowid,
                "subject": m.subject,
                "sender": m.sender,
                "sender_name": m.sender_display,
                "date": chrono::DateTime::from_timestamp(m.received_at, 0)
                    .map(|d| d.to_rfc3339())
                    .unwrap_or_default(),
                "is_read": m.is_read,
            })
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
}

async fn mail_search_once(
    mail: &MailConnector,
    sender: Option<String>,
    subject: Option<String>,
    keywords: Vec<String>,
    date_from: Option<i64>,
    date_to: Option<i64>,
    limit: usize,
) -> anyhow::Result<Vec<apple_mail_connector::MailMessage>> {
    let m = mail.clone();
    Ok(tokio::task::spawn_blocking(move || {
        m.search_messages(&MailSearchFilter {
            sender,
            subject,
            date_from,
            date_to,
            limit,
            keywords,
        })
    })
    .await??)
}

fn mail_ref_from(m: &apple_mail_connector::MailMessage) -> MailRef {
    MailRef {
        rowid: m.rowid,
        message_id: m.message_id.clone(),
        subject: m.subject.clone(),
        sender: m.sender.clone(),
        auto_open: false,
    }
}

async fn tool_mail_search(
    mail: &MailConnector,
    args: &serde_json::Value,
) -> (String, Option<MailRef>) {
    let sender = json_str_arg(args, "sender");
    let subject = json_str_arg(args, "subject");
    let keywords: Vec<String> = args["keywords"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let date_from = json_day_ts(args, "date_from", false);
    let date_to = json_day_ts(args, "date_to", true);
    let limit = args["limit"].as_u64().unwrap_or(10).min(25) as usize;

    let mut msgs = match mail_search_once(
        mail,
        sender.clone(),
        subject.clone(),
        keywords,
        date_from,
        date_to,
        limit,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return (format!("Mail search error: {e}"), None),
    };

    if msgs.is_empty() {
        if let Some(ref s) = sender {
            // LIKE can't bridge "Tomas Juricek" ↔ tomas.juricek@novem.sk —
            // retry with the sender split into AND-ed keyword tokens.
            let toks: Vec<String> = s
                .split(['@', '.', ' ', ',', '<', '>'])
                .map(str::trim)
                .filter(|t| t.len() >= 2)
                .map(str::to_lowercase)
                .collect();
            if !toks.is_empty() {
                if let Ok(v) =
                    mail_search_once(mail, None, subject.clone(), toks, date_from, date_to, limit)
                        .await
                {
                    msgs = v;
                }
            }
        }
    }

    if msgs.is_empty() {
        return ("No mail messages matched.".to_string(), None);
    }
    let mail_ref = msgs.first().map(mail_ref_from);
    (mail_headers_json(&msgs), mail_ref)
}

async fn tool_mail_list_inbox(mail: &MailConnector, args: &serde_json::Value) -> String {
    let limit = args["limit"].as_u64().unwrap_or(10).min(25) as usize;
    let unread_only = args["unread_only"].as_bool().unwrap_or(false);
    let m = mail.clone();
    match tokio::task::spawn_blocking(move || m.list_inbox(limit, unread_only)).await {
        Ok(Ok(msgs)) if msgs.is_empty() => "Inbox query returned no messages.".to_string(),
        Ok(Ok(msgs)) => mail_headers_json(&msgs),
        Ok(Err(e)) => format!("Mail error: {e}"),
        Err(e) => format!("Mail error: {e}"),
    }
}

async fn tool_mail_read(
    mail: &MailConnector,
    args: &serde_json::Value,
) -> (String, Option<MailRef>) {
    let Some(rowid) = args["rowid"].as_i64() else {
        return ("rowid is required.".to_string(), None);
    };
    let m = mail.clone();
    match tokio::task::spawn_blocking(move || m.get_message(rowid)).await {
        Ok(Ok(Some(msg))) => {
            let body: String = msg
                .body
                .as_deref()
                .unwrap_or("[message body not cached locally]")
                .chars()
                .take(4000)
                .collect();
            let date = chrono::DateTime::from_timestamp(msg.received_at, 0)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default();
            let r = mail_ref_from(&msg);
            (
                format!(
                    "From: {}\nSubject: {}\nDate: {}\n\n{}",
                    msg.sender, msg.subject, date, body
                ),
                Some(r),
            )
        }
        Ok(Ok(None)) => ("No message with that rowid.".to_string(), None),
        Ok(Err(e)) => (format!("Mail error: {e}"), None),
        Err(e) => (format!("Mail error: {e}"), None),
    }
}

async fn tool_mail_open(mail: &MailConnector, args: &serde_json::Value) -> String {
    let Some(rowid) = args["rowid"].as_i64() else {
        return "rowid is required.".to_string();
    };
    let m = mail.clone();
    let msg = match tokio::task::spawn_blocking(move || m.get_message(rowid)).await {
        Ok(Ok(Some(msg))) => msg,
        Ok(Ok(None)) => return "No message with that rowid.".to_string(),
        Ok(Err(e)) => return format!("Mail error: {e}"),
        Err(e) => return format!("Mail error: {e}"),
    };
    match apple_mail_connector::open_message(msg.message_id.as_deref(), &msg.subject, &msg.sender)
        .await
    {
        Ok(_) => format!("Opened in Mail.app: {}", msg.subject),
        Err(e) => format!("Could not open Mail.app: {e}"),
    }
}

async fn tool_notes_search(notes: &NotesConnector, args: &serde_json::Value) -> String {
    let Some(query) = json_str_arg(args, "query") else {
        return "query is required.".to_string();
    };
    let limit = args["limit"].as_u64().unwrap_or(10).min(25) as usize;
    let n = notes.clone();
    match tokio::task::spawn_blocking(move || n.search_notes(&query, limit)).await {
        Ok(Ok(found)) if found.is_empty() => "No notes matched.".to_string(),
        Ok(Ok(found)) => {
            let items: Vec<serde_json::Value> = found
                .iter()
                .map(|note| {
                    serde_json::json!({
                        "coredata_id": note.coredata_id,
                        "title": note.title,
                        "snippet": note.snippet,
                        "folder": note.folder,
                        "modified": chrono::DateTime::from_timestamp(note.modified_at, 0)
                            .map(|d| d.to_rfc3339())
                            .unwrap_or_default(),
                    })
                })
                .collect();
            serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
        }
        Ok(Err(e)) => format!("Notes error: {e}"),
        Err(e) => format!("Notes error: {e}"),
    }
}

async fn tool_notes_read(notes: &NotesConnector, args: &serde_json::Value) -> String {
    let Some(id) = json_str_arg(args, "coredata_id") else {
        return "coredata_id is required.".to_string();
    };
    match notes.get_note_body(&id).await {
        Ok(Some(body)) => body.chars().take(4000).collect(),
        Ok(None) => "Note body unavailable (locked or missing).".to_string(),
        Err(e) => format!("Notes error: {e}"),
    }
}

async fn tool_whatsapp_list_chats(wa: &WhatsappConnector, args: &serde_json::Value) -> String {
    let limit = args["limit"].as_u64().unwrap_or(20).min(50) as usize;
    match wa.list_chats(limit).await {
        Ok(chats) if chats.is_empty() => "No WhatsApp chats found.".to_string(),
        Ok(chats) => {
            let items: Vec<serde_json::Value> = chats
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "chat_id": c.id,
                        "name": c.name,
                        "is_group": c.is_group,
                        "unread": c.unread_count,
                        "last_message": c.last_message_preview,
                    })
                })
                .collect();
            serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
        }
        Err(e) => format!("WhatsApp unavailable: {e}"),
    }
}

async fn tool_whatsapp_chat_messages(
    wa: &WhatsappConnector,
    args: &serde_json::Value,
) -> (String, Option<WhatsappRef>) {
    let Some(chat_id) = json_str_arg(args, "chat_id") else {
        return ("chat_id is required.".to_string(), None);
    };
    let limit = args["limit"].as_u64().unwrap_or(20).min(50) as usize;
    match wa.chat_messages(&chat_id, limit, None).await {
        Ok(msgs) if msgs.is_empty() => ("No messages in that chat.".to_string(), None),
        Ok(msgs) => {
            let items: Vec<serde_json::Value> = msgs
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "from": if m.from_me { "me" } else { m.from.as_str() },
                        "body": m.body.chars().take(500).collect::<String>(),
                        "timestamp": m.timestamp,
                        "has_media": m.has_media,
                    })
                })
                .collect();
            let wa_ref = WhatsappRef {
                chat_id: chat_id.clone(),
                contact_name: None,
                snippet: msgs.last().map(|m| m.body.chars().take(120).collect()),
                source: "tool_loop".to_string(),
                last_message_timestamp: msgs.last().map(|m| m.timestamp),
            };
            (
                serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string()),
                Some(wa_ref),
            )
        }
        Err(e) => (format!("WhatsApp unavailable: {e}"), None),
    }
}

async fn tool_odoo(
    odoo: &OdooConnector,
    tool: &str,
    args: &serde_json::Value,
) -> (String, Option<OdooRecordRef>) {
    let limit = args["limit"].as_u64().unwrap_or(10).min(25) as u32;
    let open_only = args["open_only"].as_bool().unwrap_or(false);
    let result = match tool {
        "odoo_search_partners" => match json_str_arg(args, "query") {
            Some(q) => odoo.search_partners(&q, limit).await.map(Some),
            None => return ("query is required.".to_string(), None),
        },
        "odoo_my_invoices" => odoo.my_invoices(open_only, limit).await.map(Some),
        "odoo_my_helpdesk_tickets" => odoo.my_helpdesk_tickets(open_only, limit).await.map(Some),
        "odoo_get_record" => {
            let model = json_str_arg(args, "model").unwrap_or_default();
            let Some(id) = args["id"].as_i64() else {
                return ("id is required.".to_string(), None);
            };
            odoo.get_record(&model, id).await
        }
        _ => return (format!("Unknown Odoo tool: {tool}"), None),
    };
    match result {
        Ok(Some(r)) => {
            let odoo_ref = r
                .first_id
                .map(|id| odoo.record_ref(&r.model, id, r.first_name.as_deref().unwrap_or("")));
            (r.text, odoo_ref)
        }
        Ok(None) => ("Record not found.".to_string(), None),
        Err(e) => (format!("Odoo error: {e}"), None),
    }
}
