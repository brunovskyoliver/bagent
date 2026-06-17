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
use bagent_agent::{
    has_explicit_trigger, ContextPlanner, CorrectionClassifier, DirectiveExtractor,
    MailIntentClassifier, MemoryExtractor, OdooAction, OdooIntentClassifier, PromptBuilder,
    PromptTrace, ScreenIntentClassifier, SelectedSkill, TaskRater, WhatsappAction,
    WhatsappIntentClassifier, WindowIntentClassifier,
};
use bagent_attachments::extract as extract_attachment;
use bagent_memory::{selector as memory_selector, InsertParams, MemoryStore, RetrieveQuery};
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
use odoo_connector::{OdooConfig, OdooConnector, OdooRecordRef};
use ollama_connector::{
    ChatTurn as OllamaChatTurn, Message, OllamaClient, DEFAULT_BASE_URL, DEFAULT_EMBED_MODEL,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, convert::Infallible, io::Write, path::PathBuf, sync::Arc};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use whatsapp_connector::{
    WhatsappConfig, WhatsappConnectionStatus, WhatsappConnector, WhatsappSendTarget,
};

mod embedded {
    refinery::embed_migrations!("migrations");
}

const SUMMARIZE_THRESHOLD: usize = 60;
const KEEP_RECENT: usize = 20;
const MAX_HISTORY: usize = 20; // 40 turns → ~2000 token prefill; 20 keeps TTFT <1.5s

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
    /// Context planner for the planning layer (deterministic + LLM fallback).
    context_planner: Arc<ContextPlanner>,
    /// Deterministic task rater — classifies local vs Codex tasks.
    task_rater: Arc<TaskRater>,
    /// Codex external-reasoning connector (None when binary not found).
    codex: Option<CodexConnector>,
    /// Odoo connector — in-memory only; API key never written to disk.
    /// Swift re-pushes credentials from Keychain on each launch.
    odoo: Arc<RwLock<Option<OdooConnector>>>,
    /// WhatsApp Web bridge connector. Always present; owns the bridge subprocess.
    /// Bridge is started/stopped explicitly via `/whatsapp/start` and `/whatsapp/stop`.
    whatsapp: Arc<WhatsappConnector>,
}

type OllamaMsg = Message;

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
    #[serde(default)]
    history: Vec<OllamaMsg>,
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let data_dir = app_data_dir();
    std::fs::create_dir_all(&data_dir)?;
    let attachments_dir = data_dir.join("attachments");
    std::fs::create_dir_all(&attachments_dir)?;
    let debug_dir = data_dir.join("debug");
    std::fs::create_dir_all(&debug_dir)?;

    let mut conn = Connection::open(data_dir.join("bagent.db"))?;
    embedded::migrations::runner()
        .run(&mut conn)
        .map_err(|e| anyhow::anyhow!("migration error: {e}"))?;
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

    // Startup: warm both chat model and embed model into GPU memory so first user
    // query doesn't pay cold-load cost (~5-10s for 4.7GB + 1.2GB models).
    // Fires after a short delay to avoid competing with server startup.
    {
        let warmup_ollama = ollama.clone();
        let warmup_chat_model = "qwen2.5:7b".to_string();
        let warmup_embed_model = ollama_connector::DEFAULT_EMBED_MODEL.to_string();
        tokio::spawn(async move {
            // No delay — start warming immediately. The HTTP server is up by this point.
            // Both models load in parallel: sequential was the bug (bge-m3 only started
            // after qwen2.5:7b finished ~5s, leaving a cold-embed window).
            let ollama_chat = warmup_ollama.clone();
            let ollama_embed = warmup_ollama.clone();
            let (r_chat, r_embed) = tokio::join!(
                ollama_chat.generate_raw(&warmup_chat_model, ".", 0.0),
                ollama_embed.embed(&warmup_embed_model, "warmup"),
            );
            if r_chat.is_ok() {
                tracing::info!("warmup: chat model loaded");
            }
            if r_embed.is_ok() {
                tracing::info!("warmup: embed model loaded");
            }
        });
    }

    // Startup: import any markdown mirror files changed since last run
    {
        let mirror_memory = memory.clone();
        tokio::spawn(async move {
            mirror_memory.scan_and_import_mirror().await;
        });
    }

    // Startup: backfill embeddings for chat_turns missing from embeddings table
    {
        let backfill_memory = memory.clone();
        let backfill_db_path = data_dir.join("bagent.db");
        tokio::spawn(async move {
            // Small delay to let the server start before loading the embed model
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let conn = match rusqlite::Connection::open(&backfill_db_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("backfill: failed to open db: {e}");
                    return;
                }
            };
            let ids: Vec<(String, String)> = {
                let mut stmt = match conn.prepare(
                    "SELECT id, content FROM chat_turns \
                     WHERE id NOT IN (SELECT item_id FROM embeddings WHERE source='chat_turn') \
                     AND role IN ('user','assistant') \
                     LIMIT 200",
                ) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .ok()
                    .map(|rows| rows.flatten().collect())
                    .unwrap_or_default()
            };
            tracing::info!("backfill: embedding {} chat_turns", ids.len());
            for (id, content) in ids {
                if let Err(e) = backfill_memory.embed_chat_turn(&id, &content).await {
                    tracing::debug!("backfill embed error: {e}");
                }
                // Throttle to avoid hammering Ollama
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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

    let default_model =
        std::env::var("BAGENT_DEFAULT_MODEL").unwrap_or_else(|_| "qwen2.5:7b".to_string());
    let classifier_model =
        std::env::var("BAGENT_CLASSIFIER_MODEL").unwrap_or_else(|_| "qwen2.5:0.5b".to_string());
    let vision_model =
        std::env::var("BAGENT_VISION_MODEL").unwrap_or_else(|_| "qwen2.5vl:7b".to_string());

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

    let context_planner = Arc::new(ContextPlanner::new(
        ollama.clone(),
        classifier_model.clone(),
    ));

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

    // Odoo connector — starts unconfigured; Swift pushes creds from Keychain via POST /odoo/config.
    let odoo: Arc<RwLock<Option<OdooConnector>>> = Arc::new(RwLock::new(None));

    // WhatsApp connector — always present; bridge not started until POST /whatsapp/start.
    let whatsapp = Arc::new(WhatsappConnector::new(WhatsappConfig::default()));

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
        context_planner,
        task_rater,
        codex,
        odoo,
        whatsapp,
    };

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
        .route("/debug/context-plan", post(debug_context_plan))
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

    axum::serve(listener, app).await?;
    Ok(())
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

#[derive(Deserialize)]
struct ContextPlanRequest {
    message: String,
    #[serde(default = "default_und")]
    language: String,
    #[serde(default)]
    has_mail_ctx: bool,
}

async fn debug_context_plan(
    State(state): State<AppState>,
    Json(req): Json<ContextPlanRequest>,
) -> impl IntoResponse {
    let lang = if req.language == "auto" {
        if req.message.chars().any(|c| "áčďéíľĺňóôŕšťúýž".contains(c)) {
            "sk"
        } else {
            "en"
        }
    } else {
        &req.language
    };

    let plan = state
        .context_planner
        .plan(&req.message, lang, req.has_mail_ctx)
        .await;

    // Run skill selection for the response
    let selected_skills =
        skill_selector::select(&plan.candidate_skill_names, &state.skills, &req.message);
    let selected_skill_names: Vec<&str> = selected_skills.iter().map(|s| s.name.as_str()).collect();

    // Run memory selection (dry run — no updates to use_count)
    let selected_memory_ids: Vec<String> =
        if plan.needs_memory && !plan.memory_namespaces.is_empty() {
            let ns_refs: Vec<&str> = plan.memory_namespaces.iter().map(|s| s.as_str()).collect();
            let kind_refs: Vec<&str> = plan.memory_kinds.iter().map(|s| s.as_str()).collect();
            state
                .memory
                .retrieve_filtered(RetrieveQuery {
                    query: &req.message,
                    namespaces: &ns_refs,
                    kinds: &kind_refs,
                    k: 6,
                    max_per_namespace: 3,
                    score_threshold: 0.0,
                    allow_sensitive: false,
                })
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|h| h.item.id)
                .collect()
        } else {
            vec![]
        };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "task_type": plan.task_type,
            "response_language_hint": format!("{:?}", plan.response_language_hint),
            "needs_memory": plan.needs_memory,
            "memory_namespaces": plan.memory_namespaces,
            "memory_kinds": plan.memory_kinds,
            "needs_conversation_recall": plan.needs_conversation_recall,
            "candidate_skill_names": plan.candidate_skill_names,
            "selected_skill_names": selected_skill_names,
            "selected_memory_ids": selected_memory_ids,
            "confidence": plan.confidence,
        })),
    )
}

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
    preview_text(&format!("{layers}; {recall}"), 180)
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
    let classifier_model = state.classifier_model.clone();
    let intent_model = state.classifier_model.clone();
    let db = state.db.clone();
    let ollama = state.ollama.clone();
    let user_message = req.message.clone();
    let mail = state.mail.clone();
    let notes = state.notes.clone();
    let ctx_db = state.db.clone();
    let memory = state.memory.clone();
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
    let skills = state.skills.clone();
    let context_planner = state.context_planner.clone();
    let task_rater = state.task_rater.clone();
    let fs = state.fs.clone();
    let fs_exec = state.fs.clone(); // kept for action execution in handler post-classify

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

        // Check for explicit memory directive before answering
        let directive_extractor = DirectiveExtractor::new(ollama.clone(), classifier_model.clone());
        if has_explicit_trigger(&user_message) {
            if let Ok(Some(directive)) = directive_extractor.detect_and_extract(&user_message).await
            {
                if let Ok(Some(mem_id)) = memory
                    .insert_full(InsertParams {
                        namespace: &directive.namespace,
                        kind: &directive.kind,
                        language: &directive.language,
                        text: &directive.directive,
                        source: "explicit",
                        confidence: 0.95,
                        importance: 0.80,
                        sensitivity: "normal",
                        ..Default::default()
                    })
                    .await
                {
                    let ev = Event::default()
                        .data(serde_json::json!({"type":"memory_saved","id": mem_id}).to_string());
                    let _ = tx.send(Ok(ev)).await;
                }
            }
        }

        tracing::info!(
            "chat timing: directive check {}ms",
            t0.elapsed().as_millis()
        );
        // Load server-side history + session summary + last mail ref + last file ref + last odoo ref in parallel
        let (history, session_summary, last_mail_ref, last_file_ref, last_odoo_ref) = tokio::join!(
            async {
                if req.history.is_empty() {
                    load_session_history(&db, &session_id).await
                } else {
                    prepare_history(&ollama, &model, req.history).await
                }
            },
            load_session_summary(&db, &session_id),
            load_last_mail_ref(&db, &session_id),
            load_last_file_ref(&db, &session_id),
            load_last_odoo_ref(&db, &session_id),
        );
        tracing::info!("chat timing: history loaded {}ms", t0.elapsed().as_millis());

        // Detect language (simple heuristic: SK diacritics present?)
        let lang = if user_message.chars().any(|c| "áčďéíľĺňóôŕšťúýž".contains(c)) {
            "sk"
        } else {
            "en"
        };

        // Determine which tools are needed and gate via rules engine
        let low = user_message.to_lowercase();
        let needs_mail = [
            "email",
            "mail",
            "správ",
            "inbox",
            "schránk",
            "doručen",
            "posledn",
            "prečítaj",
            "read",
            "sender",
            "odosielate",
            "nazvom",
            "názvom",
            "mailbox",
            "prilohu",
            "prílohu",
        ]
        .iter()
        .any(|kw| low.contains(kw));
        let needs_notes = ["poznámk", "note", "zápis", "zapisal", "napísal"]
            .iter()
            .any(|kw| low.contains(kw));
        let needs_file = fs.is_some() && [
            "nájdi", "vyhľadaj", "kde mám", "kde je súbor", "súbor", "priečinok",
            "finder", "preview", "faktúr", "zmluv", "zmluvu", "zmluva",
            "find file", "find document", "find invoice", "find contract",
            "search files", "search documents", "search for file", "files containing",
            "open file", "open folder", "reveal in finder", "show in finder",
            "open in", "open with", "launch ", "focus finder",
        ].iter().any(|kw| low.contains(kw))
            // Also trigger when user references last found file
            || (last_file_ref.is_some() && [
                "otvor ho", "otvor ju", "otvor to", "open it", "reveal it",
                "show it", "ukáž", "ten súbor", "that file",
            ].iter().any(|kw| low.contains(kw)));

        let mut allowed_tools: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (needed, tool_name, description) in [
            (
                needs_mail,
                "mail_inbox",
                "Čítanie poštovej schránky (Apple Mail)",
            ),
            (needs_notes, "notes_list", "Čítanie poznámok (Apple Notes)"),
            (
                needs_file,
                "filesystem.search_files",
                "Vyhľadávanie lokálnych súborov",
            ),
        ] {
            if !needed {
                continue;
            }
            match rules.check(tool_name, "{}") {
                ApprovalLevel::Auto => {
                    allowed_tools.insert(tool_name.to_string());
                }
                ApprovalLevel::Ask => {
                    let approved =
                        request_tool_approval(&db, &pending_approvals, &tx, tool_name, description)
                            .await;
                    if approved {
                        allowed_tools.insert(tool_name.to_string());
                    }
                }
                ApprovalLevel::Forbidden => {
                    let _ = tx
                        .send(Ok(Event::default().data(
                            serde_json::json!({"type":"tool_blocked","tool": tool_name})
                                .to_string(),
                        )))
                        .await;
                }
            }
        }

        tracing::info!("chat timing: rules checked {}ms", t0.elapsed().as_millis());
        // Fetch live tool context (mail/notes/window) only for approved tools.
        // Filesystem turns are now handled by the agentic tool loop below.
        let (tool_ctx, mail_pdf_paths, mail_ref_opt, action_taken, odoo_ref_opt) =
            fetch_tool_context(
                &user_message,
                &history,
                last_mail_ref.as_ref(),
                last_odoo_ref.as_ref(),
                &allowed_tools,
                ctx_db,
                mail,
                notes,
                ollama.clone(),
                intent_model.clone(),
                memory.clone(),
                state.odoo.clone(),
                state.whatsapp.clone(),
                state.pending_approvals.clone(),
                state.rules.clone(),
            )
            .await;

        // Background action (e.g. workspace switch): skip LLM, emit brief confirmation.
        if let Some(action_msg) = action_taken {
            let _ = tx
                .send(Ok(Event::default().data(
                    serde_json::json!({"type": "action_taken", "message": action_msg}).to_string(),
                )))
                .await;
            let _ = tx
                .send(Ok(Event::default().data(
                    serde_json::json!({"type": "done", "session_id": session_id}).to_string(),
                )))
                .await;
            return;
        }

        // Emit mail_found before tokens so the MailRef is in place when the client
        // starts watching for the auto-open trigger.
        if let Some(ref mail_ref) = mail_ref_opt {
            let _ = tx
                .send(Ok(Event::default().data(
                    serde_json::json!({
                        "type": "mail_found",
                        "rowid": mail_ref.rowid,
                        "message_id": mail_ref.message_id,
                        "subject": mail_ref.subject,
                        "sender": mail_ref.sender,
                        "auto_open": mail_ref.auto_open,
                    })
                    .to_string(),
                )))
                .await;
            // Persist for cross-turn reference ("tento mail", "má prílohy?")
            save_last_mail_ref(&db, &session_id, mail_ref).await;
        }

        // Emit odoo_found so the client can show an "Otvoriť v Safari" button.
        if let Some(ref odoo_ref) = odoo_ref_opt {
            let _ = tx
                .send(Ok(Event::default().data(
                    serde_json::json!({
                        "type": "odoo_found",
                        "model": odoo_ref.model,
                        "record_id": odoo_ref.id,
                        "name": odoo_ref.name,
                        "url": odoo_ref.url,
                    })
                    .to_string(),
                )))
                .await;
            save_last_odoo_ref(&db, &session_id, odoo_ref).await;
        }

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

        // ── Planning layer ────────────────────────────────────────────────────
        // ContextPlanner → SkillSelector → MemorySelector (all before prompt build)
        let has_mail_ctx = tool_ctx.is_some();
        let context_plan = context_planner
            .plan(&user_message, lang, has_mail_ctx)
            .await;
        tracing::info!(
            "chat timing: context plan ready {}ms — task={} needs_memory={} skills={:?}",
            t0.elapsed().as_millis(),
            context_plan.task_type,
            context_plan.needs_memory,
            context_plan.candidate_skill_names
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
                skill_selector::select(&context_plan.candidate_skill_names, &skills, &user_message);
            bagent_selected
                .into_iter()
                .map(|s| SelectedSkill {
                    name: s.name,
                    body: s.body,
                })
                .collect()
        };

        // Select memory + corrections
        let (selected_memory, corrections, recall_candidates) = {
            let (mem_result, corr_result, recall_result) = tokio::join!(
                async {
                    if context_plan.needs_memory && !context_plan.memory_namespaces.is_empty() {
                        memory_selector::select(
                            &memory,
                            memory_selector::SelectQuery {
                                query: &user_message,
                                namespaces: &context_plan.memory_namespaces,
                                kinds: &context_plan.memory_kinds,
                                max_cards: None,
                            },
                        )
                        .await
                        .unwrap_or_default()
                    } else {
                        vec![]
                    }
                },
                async {
                    // Corrections and glossary always retrieved when memory is needed
                    if context_plan.needs_memory {
                        memory
                            .retrieve(
                                &user_message,
                                &["sk_glossary", "correction", "corrections", "negative_rules"],
                                6,
                            )
                            .await
                            .unwrap_or_default()
                    } else {
                        vec![]
                    }
                },
                async {
                    // Recall candidates fetched always for debug trace; injected only when planned
                    memory
                        .retrieve_turn_candidates(&user_message, Some(&session_id), 3)
                        .await
                        .unwrap_or_default()
                },
            );
            (mem_result, corr_result, recall_result)
        };

        tracing::info!(
            "chat timing: memory selected {}ms — {} cards, {} corrections, recall={}",
            t0.elapsed().as_millis(),
            selected_memory.len(),
            corrections.len(),
            context_plan.needs_conversation_recall
        );

        let context_plan_json = serde_json::to_value(&context_plan).ok();

        // ── Build layered prompt ──────────────────────────────────────────────
        let prompt_trace_id = Uuid::new_v4().to_string();
        let mut prompt_trace: Option<PromptTrace> = None;
        let mut messages = match prompt_builder
            .build(
                &user_message,
                lang,
                &context_plan.response_language_hint,
                &selected_skills,
                &selected_memory,
                &corrections,
                tool_ctx,
                att_data.ctx,
                history,
                session_summary,
                recall_candidates,
                context_plan.needs_conversation_recall,
                context_plan_json,
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

        // Persist user turn + attachment links
        {
            let turn_id = Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            if let Ok(db) = db.try_lock() {
                let _ = db.execute(
                    "INSERT INTO chat_turns (id, session_id, role, content, language, model, created_at) \
                     VALUES (?1,?2,'user',?3,?4,?5,?6)",
                    rusqlite::params![turn_id, session_id, user_message, lang, effective_model, now],
                );
                for att_id in &att_data.turn_ids {
                    let _ = db.execute(
                        "INSERT OR IGNORE INTO chat_turn_attachments (chat_turn_id, attachment_id) VALUES (?1, ?2)",
                        rusqlite::params![turn_id, att_id],
                    );
                }
            }
        }

        let prompt_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        tracing::info!(
            "chat timing: prompt built {}ms — {} msgs ~{} chars ~{} tokens",
            t0.elapsed().as_millis(),
            messages.len(),
            prompt_chars,
            prompt_chars / 4
        );
        let prompt_messages_for_log = messages.clone();

        // ── Agentic file tool loop ────────────────────────────────────────────
        // For file-search turns the model drives search/read/open tool calls
        // and sees only real filesystem results — hallucination is structurally
        // impossible because the model can only name files that tools returned.
        let is_file_turn = context_plan.task_type == "file_search"
            || needs_file
            || (last_file_ref.is_some() && {
                let lv = user_message.to_lowercase();
                [
                    "otvor ho",
                    "otvor ju",
                    "otvor to",
                    "open it",
                    "reveal it",
                    "show it",
                    "ukáž",
                    "ten súbor",
                    "that file",
                ]
                .iter()
                .any(|kw| lv.contains(kw))
            });

        if is_file_turn {
            if let Some(ref fs_c) = fs_exec {
                // Define filesystem tools exposed to the model
                use ollama_connector::ToolDef as OllamaToolDef;
                let fs_tools: Vec<OllamaToolDef> = vec![
                    OllamaToolDef::function(
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
                    ),
                    OllamaToolDef::function(
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
                    ),
                    OllamaToolDef::function(
                        "filesystem_open_file",
                        "Open a local file in its default application. Requires user approval.",
                        serde_json::json!({
                            "type": "object",
                            "properties": {
                                "path": {"type": "string", "description": "Absolute path to the file."}
                            },
                            "required": ["path"]
                        }),
                    ),
                    OllamaToolDef::function(
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
                    ),
                    OllamaToolDef::function(
                        "filesystem_reveal_in_finder",
                        "Reveal a local file in the macOS Finder.",
                        serde_json::json!({
                            "type": "object",
                            "properties": {
                                "path": {"type": "string"}
                            },
                            "required": ["path"]
                        }),
                    ),
                    OllamaToolDef::function(
                        "macos_open_app",
                        "Launch or focus a macOS application by name.",
                        serde_json::json!({
                            "type": "object",
                            "properties": {
                                "app": {"type": "string", "description": "App name, e.g. 'Mail', 'Finder', 'Preview'."}
                            },
                            "required": ["app"]
                        }),
                    ),
                ];

                let mut found_file_ref: Option<FileRef> = None;

                // Status hint so the UI shows activity during tool-calling rounds
                let _ = tx
                    .send(Ok(Event::default().data(
                        serde_json::json!({"type":"tool_status","message":"🔎 searching..."})
                            .to_string(),
                    )))
                    .await;

                // Tool-calling loop (max 5 rounds)
                'agent: for _round in 0..5 {
                    match ollama
                        .chat_once_with_tools(
                            effective_model.clone(),
                            messages.clone(),
                            fs_tools.clone(),
                        )
                        .await
                    {
                        Ok(OllamaChatTurn::ToolCalls(calls)) => {
                            // Append the assistant message carrying the tool calls
                            messages.push(Message::assistant_tool_calls(calls.clone()));

                            for call in &calls {
                                let fn_name = &call.function.name;
                                let args = &call.function.arguments;
                                tracing::debug!("file agent tool call: {} {:?}", fn_name, args);

                                // Dispatch the tool call
                                let tool_result: String = match fn_name.as_str() {
                                    "filesystem_search_files" => {
                                        let terms: Vec<String> = args["terms"]
                                            .as_array()
                                            .map(|a| {
                                                a.iter()
                                                    .filter_map(|v| {
                                                        v.as_str().map(|s| s.to_string())
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default();
                                        let query = terms.first().cloned().unwrap_or_default();
                                        let roots: Option<Vec<String>> =
                                            args["roots"].as_array().map(|a| {
                                                a.iter()
                                                    .filter_map(|v| {
                                                        v.as_str().map(|s| s.to_string())
                                                    })
                                                    .collect()
                                            });
                                        let extensions: Option<Vec<String>> =
                                            args["extensions"].as_array().map(|a| {
                                                a.iter()
                                                    .filter_map(|v| {
                                                        v.as_str().map(|s| s.to_string())
                                                    })
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

                                    "filesystem_read_text" => {
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

                                    tool @ ("filesystem_open_file"
                                    | "filesystem_open_file_with"
                                    | "filesystem_reveal_in_finder"
                                    | "filesystem_open_folder"
                                    | "macos_open_app"
                                    | "macos_focus_app") => {
                                        // Derive the dotted rule name from the underscore tool name
                                        let rule_name = match tool {
                                            "filesystem_open_file" => "filesystem.open_file",
                                            "filesystem_open_file_with" => {
                                                "filesystem.open_file_with"
                                            }
                                            "filesystem_reveal_in_finder" => {
                                                "filesystem.reveal_in_finder"
                                            }
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
                                                        path.as_deref()
                                                            .or(app.as_deref())
                                                            .unwrap_or("?")
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
                                            format!(
                                                "Tool {rule_name} blocked — user did not approve."
                                            )
                                        } else {
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
                                                    "macos.open_app" | "macos.focus_app" => {
                                                        if let Some(ref a) = app {
                                                            fs_open::open_app(a).await
                                                        } else {
                                                            Err(anyhow::anyhow!("no app"))
                                                        }
                                                    }
                                                    _ => Err(anyhow::anyhow!("unknown")),
                                                };
                                            match result {
                                                Ok(ref resp) => {
                                                    let path_hash = path.as_deref().map(sha256_str);
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

                                    other => {
                                        tracing::warn!("unknown file agent tool: {}", other);
                                        format!("Unknown tool: {other}")
                                    }
                                };

                                messages.push(Message::tool_result(fn_name, tool_result));
                            }
                        }

                        Ok(OllamaChatTurn::Content(_)) => {
                            // Model is done calling tools — exit to the final stream
                            break 'agent;
                        }

                        Err(e) => {
                            tracing::warn!("file agent tool loop error: {}", e);
                            break 'agent;
                        }
                    }
                } // end 'agent loop

                // Persist found file ref for cross-turn coreference
                if let Some(ref fref) = found_file_ref {
                    save_last_file_ref(&db, &session_id, fref).await;
                }

                // Final streaming answer: model has all tool results in context.
                // We call chat_stream without tools so the model answers (not calls more tools).
                tracing::info!(
                    "chat timing: file agent loop done {}ms, streaming final answer",
                    t0.elapsed().as_millis()
                );
                let token_stream_agent = ollama.chat_stream(effective_model.clone(), messages);
                tokio::pin!(token_stream_agent);

                let mut full_response = String::new();
                while let Some(result) = token_stream_agent.next().await {
                    match result {
                        Ok(token) => {
                            full_response.push_str(&token);
                            let ev = Event::default().data(
                                serde_json::json!({"type":"token","content":token}).to_string(),
                            );
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

                // Persist assistant turn
                {
                    let turn_id = Uuid::new_v4().to_string();
                    let now = chrono::Utc::now().to_rfc3339();
                    if let Ok(db) = db.try_lock() {
                        let _ = db.execute(
                            "INSERT INTO chat_turns (id, session_id, role, content, language, model, created_at) \
                             VALUES (?1,?2,'assistant',?3,?4,?5,?6)",
                            rusqlite::params![turn_id, session_id, full_response, lang, effective_model, now],
                        );
                        let _ = db.execute(
                            "INSERT INTO audit_entries (action, payload, model) VALUES (?1, ?2, ?3)",
                            rusqlite::params!["chat", &user_message, &effective_model],
                        );
                    }
                }

                let _ = tx
                    .send(Ok(Event::default().data(
                        serde_json::json!({"type":"done","session_id": session_id}).to_string(),
                    )))
                    .await;

                // Background: memory extraction + turn embedding
                let memory_extractor =
                    MemoryExtractor::new(ollama.clone(), classifier_model.clone());
                let memory_bg = memory.clone();
                let user_msg_bg = user_message.clone();
                let reply_bg = full_response.clone();
                let lang_bg = lang.to_string();
                let db_bg = db.clone();
                let turn_id_bg = {
                    let db_g = db_bg.try_lock().ok();
                    db_g.and_then(|d| {
                        d.query_row(
                            "SELECT id FROM chat_turns WHERE session_id=?1 ORDER BY rowid DESC LIMIT 1",
                            rusqlite::params![session_id],
                            |r| r.get::<_, String>(0),
                        )
                        .ok()
                    })
                    .unwrap_or_default()
                };
                tokio::spawn(async move {
                    let extract_fut =
                        memory_extractor.run(&user_msg_bg, &reply_bg, memory_bg.clone(), &lang_bg);
                    let embed_fut = memory_bg.embed_chat_turn(&turn_id_bg, &reply_bg);
                    let _ = tokio::join!(extract_fut, embed_fut);
                });

                return; // ← early return; skip the non-file single-LLM path below
            }
        }
        // ── End agentic file tool loop ────────────────────────────────────────

        // Stream response
        let token_stream = ollama.chat_stream(effective_model.clone(), messages);
        tokio::pin!(token_stream);

        let mut full_response = String::new();
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

        // Emit mail attachment chips before done so the UI can show them
        if !mail_pdf_paths.is_empty() {
            let atts: Vec<serde_json::Value> = mail_pdf_paths
                .iter()
                .map(|(fname, path)| {
                    serde_json::json!({
                        "filename": fname,
                        "path": path.to_string_lossy(),
                        "size": std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
                    })
                })
                .collect();
            let _ = tx
                .send(Ok(Event::default().data(
                    serde_json::json!({"type":"mail_attachments","attachments": atts}).to_string(),
                )))
                .await;
        }

        // Persist assistant turn
        let response_for_audit = full_response.clone();
        let turn_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        if let Ok(db) = db.try_lock() {
            let _ = db.execute(
                "INSERT INTO chat_turns (id, session_id, role, content, language, model, created_at) \
                 VALUES (?1,?2,'assistant',?3,?4,?5,?6)",
                rusqlite::params![turn_id, session_id, full_response, lang, effective_model, now],
            );
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

        // Background: correction classifier + passive memory extraction + session summarizer + turn embedding
        let correction_classifier =
            CorrectionClassifier::new(ollama.clone(), classifier_model.clone());
        let memory_extractor = MemoryExtractor::new(ollama.clone(), classifier_model.clone());
        let memory_bg = memory.clone();
        let ollama_bg = ollama.clone();
        let model_bg = effective_model.clone();
        let user_msg_bg = user_message.clone();
        let reply_bg = response_for_audit.clone();
        let lang_bg = lang.to_string();
        let session_bg = session_id.clone();
        let db_bg = db.clone();
        let turn_id_bg = turn_id.clone();
        tokio::spawn(async move {
            // Embed + correction + memory extraction all in parallel — none depends on
            // the others, and each calls a different model (bge-m3 / qwen2.5:0.5b).
            let embed_fut = memory_bg.embed_chat_turn(&turn_id_bg, &reply_bg);

            let correction_fut = {
                let mem = memory_bg.clone();
                let reply = reply_bg.clone();
                let msg = user_msg_bg.clone();
                async move {
                    if let Ok(result) = correction_classifier.classify(&reply, &msg).await {
                        if result.is_correction && result.confidence > 0.7 {
                            let text = format!(
                                "Oprava: {} → {}",
                                result.what_was_wrong.as_deref().unwrap_or("?"),
                                result.correct_behavior.as_deref().unwrap_or("?")
                            );
                            let namespace = if result.scope == "sk_lang" {
                                "sk_glossary"
                            } else {
                                "corrections"
                            };
                            let _ = mem
                                .insert_full(InsertParams {
                                    namespace,
                                    kind: "correction",
                                    language: "und",
                                    text: &text,
                                    source: "passive",
                                    confidence: result.confidence,
                                    importance: 0.75,
                                    sensitivity: "normal",
                                    ..Default::default()
                                })
                                .await;
                        }
                    }
                }
            };

            let extract_fut =
                memory_extractor.run(&user_msg_bg, &reply_bg, memory_bg.clone(), &lang_bg);

            let (embed_result, _, _) = tokio::join!(embed_fut, correction_fut, extract_fut);
            if let Err(e) = embed_result {
                tracing::debug!("chat turn embed error: {e}");
            }

            // Session summarizer: every 10 turns, regenerate sessions.summary
            let turn_count: i64 = db_bg
                .try_lock()
                .ok()
                .and_then(|db| {
                    db.query_row(
                        "SELECT COUNT(*) FROM chat_turns WHERE session_id = ?1",
                        rusqlite::params![session_bg],
                        |r| r.get(0),
                    )
                    .ok()
                })
                .unwrap_or(0);

            if turn_count > 0 && turn_count % 10 == 0 {
                // Fetch last 20 turns for summary
                let turns_text: Option<String> = db_bg.try_lock().ok().and_then(|db| {
                    let mut stmt = db
                        .prepare(
                            "SELECT role, content FROM chat_turns WHERE session_id = ?1 \
                         ORDER BY created_at DESC LIMIT 20",
                        )
                        .ok()?;
                    let rows: Vec<String> = stmt
                        .query_map(rusqlite::params![session_bg], |r| {
                            let role: String = r.get(0)?;
                            let content: String = r.get(1)?;
                            Ok(format!("[{role}]: {content}"))
                        })
                        .ok()?
                        .flatten()
                        .collect();
                    Some(rows.into_iter().rev().collect::<Vec<_>>().join("\n"))
                });

                if let Some(text) = turns_text {
                    let prompt = format!(
                        "Summarize this conversation concisely in 2-3 sentences, preserving key facts and decisions:\n{text}"
                    );
                    if let Ok(summary) = ollama_bg
                        .summarize(&model_bg, &[Message::user(prompt)])
                        .await
                    {
                        if let Ok(db) = db_bg.try_lock() {
                            let _ = db.execute(
                                "UPDATE sessions SET summary = ?1 WHERE id = ?2",
                                rusqlite::params![summary, session_bg],
                            );
                        }
                    }
                }
            }
        });
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

async fn sessions_list(State(state): State<AppState>) -> impl IntoResponse {
    let db = state.db.lock().await;
    let mut stmt = match db.prepare(
        "SELECT id, started_at, ended_at, language, summary FROM sessions ORDER BY started_at DESC LIMIT 50"
    ) {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    let sessions: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "started_at": row.get::<_, String>(1)?,
                "ended_at": row.get::<_, Option<String>>(2)?,
                "language": row.get::<_, Option<String>>(3)?,
                "summary": row.get::<_, Option<String>>(4)?,
            }))
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();
    (
        StatusCode::OK,
        Json(serde_json::json!({ "sessions": sessions })),
    )
}

async fn session_turns(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let db = state.db.lock().await;
    let mut stmt = match db.prepare(
        "SELECT id, role, content, language, model, created_at FROM chat_turns \
         WHERE session_id = ?1 ORDER BY created_at",
    ) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };
    let turns: Vec<serde_json::Value> = stmt
        .query_map(rusqlite::params![id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "role": row.get::<_, String>(1)?,
                "content": row.get::<_, String>(2)?,
                "language": row.get::<_, String>(3)?,
                "model": row.get::<_, Option<String>>(4)?,
                "created_at": row.get::<_, String>(5)?,
            }))
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();
    (StatusCode::OK, Json(serde_json::json!({ "turns": turns })))
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
    State(state): State<AppState>,
    Json(req): Json<MemoryInsertRequest>,
) -> impl IntoResponse {
    match state
        .memory
        .insert_full(InsertParams {
            namespace: &req.namespace,
            kind: &req.kind,
            language: &req.language,
            text: &req.text,
            source_ref: req.source_ref.as_deref(),
            metadata_json: req.metadata_json.as_deref(),
            expires_at: req.expires_at.as_deref(),
            source: req.source.as_deref().unwrap_or("explicit"),
            confidence: req.confidence.unwrap_or(0.9),
            importance: req.importance.unwrap_or(0.7),
            sensitivity: req.sensitivity.as_deref().unwrap_or("normal"),
            subject: req.subject.as_deref(),
        })
        .await
    {
        Ok(Some(id)) => {
            let db = state.db.lock().await;
            let _ = db.execute(
                "INSERT INTO audit_entries (action, payload, model) VALUES ('memory_save', ?1, '')",
                rusqlite::params![
                    serde_json::json!({"id": id, "kind": req.kind, "namespace": req.namespace})
                        .to_string()
                ],
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({ "id": id, "saved": true })),
            )
        }
        Ok(None) => (
            StatusCode::OK,
            Json(serde_json::json!({ "saved": false, "reason": "duplicate" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn memory_list(
    State(state): State<AppState>,
    Query(q): Query<MemorySearchQuery>,
) -> impl IntoResponse {
    let db = state.db.lock().await;
    let sql = if q.namespace.is_empty() {
        "SELECT id, namespace, kind, language, text, source_ref, created_at, use_count, \
                status, source, confidence, importance, sensitivity \
         FROM memory_items WHERE status = 'active' ORDER BY updated_at DESC LIMIT ?1"
            .to_string()
    } else {
        "SELECT id, namespace, kind, language, text, source_ref, created_at, use_count, \
                status, source, confidence, importance, sensitivity \
         FROM memory_items WHERE namespace = ?2 AND status = 'active' ORDER BY updated_at DESC LIMIT ?1"
            .to_string()
    };

    let query_fn = |row: &rusqlite::Row<'_>| {
        Ok(serde_json::json!({
            "id": row.get::<_, String>(0)?,
            "namespace": row.get::<_, String>(1)?,
            "kind": row.get::<_, String>(2)?,
            "language": row.get::<_, String>(3)?,
            "text": row.get::<_, String>(4)?,
            "source_ref": row.get::<_, Option<String>>(5)?,
            "created_at": row.get::<_, String>(6)?,
            "use_count": row.get::<_, i64>(7)?,
            "status": row.get::<_, String>(8)?,
            "source": row.get::<_, String>(9)?,
            "confidence": row.get::<_, f64>(10)?,
            "importance": row.get::<_, f64>(11)?,
            "sensitivity": row.get::<_, String>(12)?,
        }))
    };

    let items: Vec<serde_json::Value> = if q.namespace.is_empty() {
        db.prepare(&sql)
            .ok()
            .and_then(|mut s| {
                s.query_map(rusqlite::params![q.limit as i64], query_fn)
                    .ok()
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default()
    } else {
        db.prepare(&sql)
            .ok()
            .and_then(|mut s| {
                s.query_map(rusqlite::params![q.limit as i64, q.namespace], query_fn)
                    .ok()
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default()
    };

    (StatusCode::OK, Json(serde_json::json!({ "items": items })))
}

async fn memory_search(
    State(state): State<AppState>,
    Query(q): Query<MemorySearchQuery>,
) -> impl IntoResponse {
    let namespaces: Vec<&str> = if q.namespace.is_empty() {
        vec!["global", "user_pref", "sk_glossary", "correction"]
    } else {
        vec![q.namespace.as_str()]
    };
    let kind_filter: Vec<&str> = if q.kind.is_empty() {
        vec![]
    } else {
        vec![q.kind.as_str()]
    };

    match state
        .memory
        .retrieve_filtered(RetrieveQuery {
            query: &q.q,
            namespaces: &namespaces,
            kinds: &kind_filter,
            k: q.limit,
            max_per_namespace: 3,
            score_threshold: 0.0,
            allow_sensitive: false,
        })
        .await
    {
        Ok(hits) => {
            let items: Vec<serde_json::Value> = hits
                .into_iter()
                .map(|h| {
                    serde_json::json!({
                        "id": h.item.id,
                        "namespace": h.item.namespace,
                        "kind": h.item.kind,
                        "text": h.item.text,
                        "score": h.score,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "hits": items })))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn memory_delete(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.memory.delete(&id) {
        Ok(true) => {
            let db = state.db.lock().await;
            let _ = db.execute(
                "INSERT INTO audit_entries (action, payload, model) VALUES ('memory_forget', ?1, '')",
                rusqlite::params![serde_json::json!({"id": id}).to_string()],
            );
            (StatusCode::OK, Json(serde_json::json!({ "deleted": true })))
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
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

// ── Odoo handlers (Phase 6) ──────────────────────────────────────────────────

/// `POST /odoo/config` — authenticate and store the connector in-memory.
/// Doubles as the Settings "Test" button: returns version + uid on success.
async fn odoo_config_handler(
    State(state): State<AppState>,
    Json(cfg): Json<OdooConfig>,
) -> impl IntoResponse {
    match OdooConnector::connect(cfg).await {
        Ok(conn) => {
            let version = conn.server_version.clone();
            let uid = conn.uid;
            *state.odoo.write().await = Some(conn);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "version": version,
                    "uid": uid,
                })),
            )
        }
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "ok": false,
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
            "version": conn.server_version,
            "uid": conn.uid,
        })),
        None => Json(serde_json::json!({
            "configured": false,
            "connected": false,
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
        Ok(s) => Json(serde_json::json!({
            "status": s.status.to_string(),
            "connected": s.status == WhatsappConnectionStatus::Ready,
            "needs_qr": s.status == WhatsappConnectionStatus::Qr,
            "me": s.me,
            "error": s.error,
        })),
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

    // Approval modal description — shown verbatim to the user.
    // Trap #2: the audit_description passed to request_approval_core must be
    // REDACTED (preview only) to keep raw message bodies out of audit_entries.
    let approval_description = format!(
        "Odoslať WhatsApp správu — Príjemca: {}\nText: {}",
        recipient_display, &text
    );
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

    // Store the approval description (with full text) in the modal side-channel
    // so the Swift app can show it before the user decides. Here we just proceed.
    // (The full-text version was already shown via `approval_description` above in the
    //  approval request that the Swift app rendered.)

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
    memory: Arc<MemoryStore>,
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

    let rowids: Vec<i64> = new_msgs.iter().map(|m| m.rowid).collect();

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

    // Post-sync: embed new mail subjects for semantic search (best-effort, background)
    if !rowids.is_empty() {
        let memory_embed = memory.clone();
        let msgs_for_embed = new_msgs;
        tokio::spawn(async move {
            for msg in msgs_for_embed {
                let text = format!("{} {}", msg.subject, msg.sender);
                let turn_id = format!("mail:{}", msg.rowid);
                let _ = memory_embed.embed_chat_turn(&turn_id, &text).await;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        });
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

/// Extract a subject hint from natural-language patterns.
///
/// Handles: "by mal byt X", "nazvom X", "named X", quoted strings.
/// "subject X" is intentionally NOT used as a prefix — the word "subject" in
/// Slovak sentences like "subject toho mailu by mal byt X" is the topic marker,
/// not a direct prefix of the title. We catch those via "by mal byt" instead.
/// Parse an ISO date string "YYYY-MM-DD" into a [start, end) Unix-epoch second range
/// covering the whole calendar day in UTC.
fn parse_date_to_range(iso: &str) -> Option<(i64, i64)> {
    use chrono::{NaiveDate, TimeZone, Utc};
    let d = NaiveDate::parse_from_str(iso, "%Y-%m-%d").ok()?;
    let start = Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0)?).timestamp();
    let end = Utc
        .from_utc_datetime(&d.and_hms_opt(23, 59, 59)?)
        .timestamp()
        + 1;
    Some((start, end))
}

fn push_search_term(terms: &mut Vec<String>, term: Option<&str>) {
    let Some(term) = term.map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    if !terms
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(term))
    {
        terms.push(term.to_string());
    }
}

async fn search_mail_messages(
    mail: &Option<MailConnector>,
    filter: MailSearchFilter,
) -> Vec<apple_mail_connector::MailMessage> {
    if let Some(ref mc) = *mail {
        let mc = mc.clone();
        tokio::task::spawn_blocking(move || mc.search_messages(&filter))
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default()
    } else {
        vec![]
    }
}

/// Helper: fetch recent messages, merging mail_cache with live Envelope Index.
/// Always queries both sources so very-recent (not-yet-synced) mails appear.
async fn fetch_recent_mails(
    db: &Arc<Mutex<Connection>>,
    mail: &Option<MailConnector>,
    limit: usize,
) -> Vec<apple_mail_connector::MailMessage> {
    let cached: Vec<apple_mail_connector::MailMessage> =
        {
            let db_lock = db.lock().await;
            db_lock.prepare(
            "SELECT rowid, subject, sender, sender_display, received_at, is_read, mailbox_url \
             FROM mail_cache ORDER BY received_at DESC LIMIT ?1"
        )
        .and_then(|mut s| s.query_map(rusqlite::params![limit as i64], |r| {
            let display: Option<String> = r.get(3)?;
            Ok(apple_mail_connector::MailMessage {
                rowid: r.get(0)?,
                subject: r.get(1)?,
                sender: r.get(2)?,
                sender_display: display,
                received_at: r.get(4)?,
                is_read: r.get::<_, i64>(5)? != 0,
                mailbox_url: r.get(6)?,
                recipient: None,
                body: None, body_available: true, language: None, attachments: vec![],
                message_id: None,
            })
        }).map(|rows| rows.flatten().collect()))
        .unwrap_or_default()
        };

    // Always also query live Envelope Index to catch emails received since last sync.
    let live: Vec<apple_mail_connector::MailMessage> = if let Some(ref mc) = *mail {
        let mc = mc.clone();
        tokio::task::spawn_blocking(move || mc.list_inbox(limit, false))
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default()
    } else {
        vec![]
    };

    if cached.is_empty() && live.is_empty() {
        return vec![];
    }

    // Merge: live takes precedence (fresher), dedup by rowid, sort newest-first.
    let mut seen = std::collections::HashSet::new();
    let mut merged: Vec<apple_mail_connector::MailMessage> = Vec::new();
    for m in live.into_iter().chain(cached.into_iter()) {
        if seen.insert(m.rowid) {
            merged.push(m);
        }
    }
    merged.sort_by(|a, b| b.received_at.cmp(&a.received_at));
    merged.truncate(limit);
    merged
}

/// Read body + all supported attachments for a single message.
/// Injects structured context (Od/Komu/Prijaté/Predmet/Obsah) into `lines`
/// and returns (filename, on-disk-path) for every attachment written to disk.
async fn enrich_message(
    rowid: i64,
    mail: &Option<MailConnector>,
    wants_attachment: bool,
    lines: &mut Vec<String>,
) -> Vec<(String, std::path::PathBuf)> {
    let Some(ref mc) = *mail else { return vec![] };
    let mc_clone = mc.clone();
    let full = {
        let mc2 = mc_clone.clone();
        tokio::task::spawn_blocking(move || mc2.get_message(rowid))
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten()
    };
    let Some(mut full_msg) = full else {
        return vec![];
    };
    let subject_for_fallback = full_msg.subject.clone();

    // Format datetime as "DD.MM.YYYY HH:MM" in the system local timezone.
    let dt = {
        use chrono::{DateTime, Local, Utc};
        DateTime::<Utc>::from_timestamp(full_msg.received_at, 0)
            .map(|d| d.with_timezone(&Local).format("%d.%m.%Y %H:%M").to_string())
            .unwrap_or_else(|| full_msg.received_at.to_string())
    };

    // Build the Od/Komu/Prijaté/Predmet header
    let sender_fmt = match &full_msg.sender_display {
        Some(name) if !name.is_empty() => format!("{} <{}>", name, full_msg.sender),
        _ => full_msg.sender.clone(),
    };
    // Try to extract user email from mailbox URL (e.g. "imap://user@domain/...") before falling back.
    let komu_fallback: String;
    let komu = if let Some(addr) = full_msg.recipient.as_deref() {
        addr
    } else {
        komu_fallback = full_msg
            .mailbox_url
            .split("://")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .filter(|s| s.contains('@'))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "tvoja schránka".to_string());
        &komu_fallback
    };

    // Strict guard: LLM must not modify, invent, or cross-contaminate with past context.
    lines.push(
        "⚠️ SYSTÉMOVÁ INŠTRUKCIA: Nasledujúci email je skutočný, doslovný obsah zo schránky. \
         Zobraz hlavičku a telo PRESNE ako je — nič nevymýšľaj, nedoplňaj, nenahrádzaj prázdne \
         polia vlastnými odhadmi, nemiešaj s inými rozhovormi ani kontextami."
            .to_string(),
    );

    // Use markdown hard line breaks (two trailing spaces) so each field renders on its own line.
    lines.push(format!(
        "Našiel som email:  \n**Od:** {sender_fmt}  \n**Komu:** {komu}  \n**Prijaté:** {dt}  \n**Predmet:** {}",
        full_msg.subject
    ));

    // Body
    if full_msg.body.is_none() {
        full_msg.body = apple_mail_connector::body_via_applescript(&subject_for_fallback).await;
    }

    let mut att_files: Vec<(String, std::path::PathBuf)> = Vec::new();

    if wants_attachment && !full_msg.attachments.is_empty() {
        // Extract all non-image attachments
        for att_meta in &full_msg.attachments {
            if att_meta.mimetype.starts_with("image/") {
                continue;
            }
            let mc2 = mc_clone.clone();
            let part_idx = att_meta.part_index;
            let filename = att_meta.filename.clone();
            let mime = att_meta.mimetype.clone();
            let bytes_result =
                tokio::task::spawn_blocking(move || mc2.get_message_attachment(rowid, part_idx))
                    .await
                    .ok()
                    .and_then(|r| r.ok());

            if let Some((_, bytes)) = bytes_result {
                let tmp = std::env::temp_dir().join(format!("bagent_mail_{}", &filename));
                if std::fs::write(&tmp, &bytes).is_ok() {
                    let extracted = extract_attachment(&tmp, &mime)
                        .ok()
                        .and_then(|r| r.extracted_text);
                    if let Some(text) = extracted {
                        lines.push(format!("\n\n**Obsah prílohy ({filename}):**\n\n{text}"));
                    } else {
                        lines.push(format!(
                            "\n\n**Príloha:** {filename} (obsah nedostupný na analýzu)"
                        ));
                    }
                    att_files.push((filename, tmp));
                }
            }
        }
    } else {
        // No attachment requested — show body
        if let Some(ref body) = full_msg.body {
            let truncated = if body.len() > 2000 {
                format!("{}…[skrátené]", &body[..body.floor_char_boundary(2000)])
            } else {
                body.clone()
            };
            lines.push(format!("\n\n**Obsah:**\n\n{truncated}"));
        } else {
            lines.push("\n\n**Obsah:** TELO EMAILU SA NEPODARILO NAČÍTAŤ. Toto nie je šablóna — nič nevymýšľaj ani nedoplňaj.".to_string());
        }
    }

    att_files
}

/// Detects intent with an LLM classifier and pre-fetches the right Mail / Notes
/// data so the response LLM can answer with real facts.
///
/// Returns `(tool_ctx, pdf_paths, mail_ref)`:
/// - `tool_ctx`: optional text block injected into the LLM prompt.
/// - `pdf_paths`: (filename, path) pairs for mail attachment chips.
/// - `mail_ref`: stable reference to the best-matched mail message, used to
///   render the "Otvoriť mail" button in the UI.
/// Format the last `max_turns` user/assistant messages as a compact snippet
/// for coreference resolution in intent classifiers.
fn format_history_snippet(history: &[Message], max_turns: usize) -> String {
    const MAX_CHARS: usize = 200;
    history
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .rev()
        .take(max_turns)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|m| {
            let label = if m.role == "user" {
                "[User]"
            } else {
                "[Assistant]"
            };
            let body: String = m.content.chars().take(MAX_CHARS).collect();
            format!("{}: {}", label, body)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn fetch_tool_context(
    message: &str,
    history: &[Message],
    last_mail_ref: Option<&MailRef>,
    last_odoo_ref: Option<&OdooRecordRef>,
    allowed_tools: &std::collections::HashSet<String>,
    db: Arc<Mutex<Connection>>,
    mail: Option<MailConnector>,
    notes: Option<NotesConnector>,
    ollama: OllamaClient,
    model: String,
    memory: Arc<MemoryStore>,
    odoo: Arc<RwLock<Option<OdooConnector>>>,
    whatsapp: Arc<WhatsappConnector>,
    pending_approvals: Arc<std::sync::Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    rules: Arc<RuleEngine>,
) -> (
    Option<String>,
    Vec<(String, std::path::PathBuf)>,
    Option<MailRef>,
    Option<String>,        // action_taken: Some(msg) skips LLM
    Option<OdooRecordRef>, // top Odoo record found this turn
) {
    let low = message.to_lowercase();

    // Cheap keyword gate — only run the LLM classifier when the turn looks mail-related.
    let looks_like_mail = allowed_tools.contains("mail_inbox")
        && (["email", "mail", "správ", "inbox", "schránk", "doručen",
             "posledn", "prečítaj", "read", "sender", "odosielate",
             "nazvom", "názvom", "mailbox", "prilohu", "prílohu",
             "predmet", "pošt", "message", "odoslan", "poslany", "prijat"]
             .iter().any(|kw| low.contains(kw))
            // Also trigger when user references the last found mail
            || (last_mail_ref.is_some()
                && ["tento mail", "táto správa", "ten mail", "tú správu",
                    "this mail", "this email", "its attachment", "tej správy",
                    "prilohy", "prílohy", "has it", "má prílohy", "ma prilohy"]
                    .iter().any(|kw| low.contains(kw))));

    // On battery: sync on-demand when user asks about mail (background sync is paused)
    if looks_like_mail && !is_on_ac_power() {
        if let Some(ref mc) = mail {
            tracing::info!("mail on-demand sync (battery)");
            let _ = mail_sync_inner(db.clone(), mc.clone(), memory.clone()).await;
        }
    }

    let wants_notes = allowed_tools.contains("notes_list")
        && ["poznámk", "note", "zápis", "zapisal", "napísal"]
            .iter()
            .any(|kw| low.contains(kw));

    let history_snippet = {
        let base = format_history_snippet(history, 4);
        let with_mail = match last_mail_ref {
            Some(r) => format!(
                "[LastFoundMail]: rowid={} sender=\"{}\" subject=\"{}\"\n{}",
                r.rowid, r.sender, r.subject, base
            ),
            None => base,
        };
        match last_odoo_ref {
            Some(r) => format!(
                "[LastFoundOdooRecord]: model=\"{}\" id={} name=\"{}\"\n{}",
                r.model, r.id, r.name, with_mail
            ),
            None => with_mail,
        }
    };

    let mut parts: Vec<String> = Vec::new();
    let mut pdf_paths: Vec<(String, std::path::PathBuf)> = Vec::new();
    let mut found_mail_ref: Option<MailRef> = None;
    let mut found_odoo_ref: Option<OdooRecordRef> = None;

    // ── Mail ─────────────────────────────────────────────────────────────────
    if looks_like_mail {
        let intent = MailIntentClassifier::new(ollama.clone(), model.clone())
            .classify(message, &history_snippet)
            .await
            .unwrap_or_default();

        tracing::debug!("mail_intent: {:?}", intent);

        // Safety-net: if classifier said list_recent but filters are present → treat as search.
        // This catches the common mis-classification of "recent mails from <sender>".
        let effective_action = if intent.action == "list_recent"
            && (intent.sender.is_some() || intent.subject.is_some() || !intent.keywords.is_empty())
        {
            "search"
        } else {
            intent.action.as_str()
        };

        match effective_action {
            "none" => {}

            "list_recent" => {
                let msgs = fetch_recent_mails(&db, &mail, 10).await;
                if !msgs.is_empty() {
                    let lines: Vec<String> = msgs
                        .iter()
                        .map(|m| {
                            let from = m.sender_display.as_deref().unwrap_or(&m.sender);
                            let status = if m.is_read { "✓" } else { "●" };
                            format!(
                                "  {status} [{}] Od: {} | Predmet: {}",
                                relative_date(m.received_at),
                                from,
                                m.subject
                            )
                        })
                        .collect();
                    parts.push(format!(
                        "Posledné emaily (Apple Mail):\n{}",
                        lines.join("\n")
                    ));
                } else {
                    parts
                        .push("Posledné emaily (Apple Mail): žiadne správy nenájdené.".to_string());
                }
            }

            "search" | "read_attachment" | "open" => {
                let is_attachment = intent.action == "read_attachment" || intent.wants_attachment;
                let wants_open = intent.action == "open"
                    || ["otvor", "otvoriť", "open it", "show me", "ukáž mi"]
                        .iter()
                        .any(|kw| low.contains(kw));

                // Short-circuit: "má tento mail prílohy?" with no new search filters
                // → use the last found mail's rowid directly, skip re-search.
                let no_search_filters = intent.sender.is_none()
                    && intent.subject.is_none()
                    && intent.date.is_none()
                    && intent.keywords.is_empty();
                if is_attachment && no_search_filters {
                    if let Some(ref lmr) = last_mail_ref {
                        tracing::info!(
                            "attachment short-circuit via last_mail_ref rowid={}",
                            lmr.rowid
                        );
                        if let Some(ref mc) = mail {
                            let mc2 = mc.clone();
                            let rowid = lmr.rowid;
                            // get_message populates attachments from emlx parsing
                            let full_msg =
                                tokio::task::spawn_blocking(move || mc2.get_message(rowid))
                                    .await
                                    .ok()
                                    .and_then(|r| r.ok())
                                    .flatten();

                            let attach_list = full_msg
                                .as_ref()
                                .map(|m| m.attachments.clone())
                                .unwrap_or_default();

                            if attach_list.is_empty() {
                                parts.push(format!(
                                    "Email \"{}\" (od: {}) nemá žiadne prílohy.",
                                    lmr.subject, lmr.sender
                                ));
                            } else {
                                let att_lines: Vec<String> = attach_list
                                    .iter()
                                    .map(|a| {
                                        format!("  • {} ({}, {} B)", a.filename, a.mimetype, a.size)
                                    })
                                    .collect();
                                parts.push(format!(
                                    "Prílohy emailu \"{}\" (od: {}):\n{}",
                                    lmr.subject,
                                    lmr.sender,
                                    att_lines.join("\n")
                                ));
                                // Stage PDF attachment paths for chips
                                for (idx, a) in attach_list.iter().enumerate() {
                                    if a.mimetype.contains("pdf") {
                                        if let Some(ref mc3) = mail {
                                            let mc3 = mc3.clone();
                                            let rid = lmr.rowid;
                                            if let Ok(Some((_, bytes))) =
                                                tokio::task::spawn_blocking(move || {
                                                    mc3.get_message_attachment(rid, idx).ok()
                                                })
                                                .await
                                            {
                                                let tmp = std::env::temp_dir().join(format!(
                                                    "bagent_att_{}_{}.pdf",
                                                    rid, idx
                                                ));
                                                if std::fs::write(&tmp, &bytes).is_ok() {
                                                    pdf_paths.push((a.filename.clone(), tmp));
                                                }
                                            }
                                        }
                                    }
                                }
                                found_mail_ref = Some(MailRef {
                                    rowid: lmr.rowid,
                                    message_id: lmr.message_id.clone(),
                                    subject: lmr.subject.clone(),
                                    sender: lmr.sender.clone(),
                                    auto_open: false,
                                });
                            }
                        }
                        // Done — skip the normal search path
                    }
                }

                // Only run the full search if we didn't short-circuit above
                let already_handled = is_attachment && no_search_filters && last_mail_ref.is_some();
                if !already_handled {
                    let (date_from, date_to) = intent
                        .date
                        .as_deref()
                        .and_then(parse_date_to_range)
                        .map(|(s, e)| (Some(s), Some(e)))
                        .unwrap_or((None, None));

                    // When user says "recent"/"posledné"/"nové" but no explicit date, add a
                    // 30-day window so we don't scan the entire inbox history.
                    let date_from = date_from.or_else(|| {
                        let has_recent_word =
                            ["recent", "posledn", "nové", "najnov", "latest", "new"]
                                .iter()
                                .any(|kw| low.contains(kw));
                        if has_recent_word && intent.date.is_none() {
                            Some(chrono::Utc::now().timestamp() - 30 * 24 * 3600)
                        } else {
                            None
                        }
                    });

                    // Stop-words that should never be used as SQL search filters.
                    let mail_stopwords = [
                        "recent", "mail", "email", "new", "latest", "inbox", "nové", "posledn",
                        "správ", "schránk",
                    ];
                    let meaningful_keywords: Vec<String> = intent
                        .keywords
                        .iter()
                        .filter(|kw| {
                            !mail_stopwords
                                .iter()
                                .any(|sw| kw.to_lowercase().contains(sw))
                        })
                        .cloned()
                        .collect();

                    // If LLM put the search term in keywords instead of sender/subject,
                    // promote the first meaningful keyword to sender as a fallback.
                    let effective_sender = intent.sender.clone().or_else(|| {
                        if intent.subject.is_none() && !meaningful_keywords.is_empty() {
                            Some(meaningful_keywords[0].clone())
                        } else {
                            None
                        }
                    });

                    // Only use keywords as SQL filters when sender AND subject are both known
                    // (avoids false-positive AND clauses filtering out valid results).
                    let filter_keywords: Vec<String> =
                        if effective_sender.is_some() || intent.subject.is_some() {
                            vec![]
                        } else {
                            meaningful_keywords.clone()
                        };

                    let filter = MailSearchFilter {
                        sender: effective_sender,
                        subject: intent.subject.clone(),
                        date_from,
                        date_to,
                        limit: 10,
                        keywords: filter_keywords,
                    };

                    let had_date_filter = filter.date_from.is_some() || filter.date_to.is_some();
                    let mut broad_terms = Vec::new();
                    push_search_term(&mut broad_terms, filter.sender.as_deref());
                    push_search_term(&mut broad_terms, filter.subject.as_deref());
                    for kw in &meaningful_keywords {
                        push_search_term(&mut broad_terms, Some(kw));
                    }

                    let mut msgs = search_mail_messages(&mail, filter.clone()).await;

                    // Fallback 1: retry ambiguous company/person terms across sender and subject.
                    // Example: "moneys3" may be classified as sender, while Apple Mail stores
                    // "Money S3" in display name or subject.
                    if msgs.is_empty() && !broad_terms.is_empty() {
                        let broad = MailSearchFilter {
                            sender: None,
                            subject: None,
                            date_from: filter.date_from,
                            date_to: filter.date_to,
                            limit: 10,
                            keywords: broad_terms.clone(),
                        };
                        msgs = search_mail_messages(&mail, broad).await;
                    }

                    // Fallback 2: if no results and we had a date filter, retry without it.
                    if msgs.is_empty() && had_date_filter {
                        let wider = MailSearchFilter {
                            sender: filter.sender.clone(),
                            subject: filter.subject.clone(),
                            date_from: None,
                            date_to: None,
                            limit: 10,
                            keywords: filter.keywords.clone(),
                        };
                        msgs = search_mail_messages(&mail, wider).await;
                    }

                    // Fallback 3: combine both relaxations when each one alone failed.
                    if msgs.is_empty() && had_date_filter && !broad_terms.is_empty() {
                        let wider_broad = MailSearchFilter {
                            sender: None,
                            subject: None,
                            date_from: None,
                            date_to: None,
                            limit: 10,
                            keywords: broad_terms,
                        };
                        msgs = search_mail_messages(&mail, wider_broad).await;
                    }

                    let msgs = msgs;

                    if msgs.is_empty() {
                        let mut why = Vec::new();
                        if let Some(ref s) = intent.sender {
                            why.push(format!("odosielateľ: {s}"));
                        }
                        if let Some(ref s) = intent.subject {
                            why.push(format!("predmet: {s}"));
                        }
                        if let Some(ref d) = intent.date {
                            why.push(format!("dátum: {d}"));
                        }
                        parts.push(format!(
                            "Vyhľadávanie v Apple Mail ({}) — žiadny email nenájdený. \
                         Email neexistuje v lokálnej schránke alebo nebol stiahnutý cez IMAP.",
                            if why.is_empty() {
                                "bez filtrov".to_string()
                            } else {
                                why.join(", ")
                            }
                        ));
                    } else {
                        let mut lines: Vec<String> = msgs
                            .iter()
                            .map(|m| {
                                let from = m.sender_display.as_deref().unwrap_or(&m.sender);
                                let status = if m.is_read { "✓" } else { "●" };
                                format!(
                                    "  {status} [{}] Od: {} <{}> | Predmet: {}",
                                    relative_date(m.received_at),
                                    from,
                                    m.sender,
                                    m.subject
                                )
                            })
                            .collect();

                        // Find best match: prefer message whose body contains a keyword.
                        let mut best_rowid = msgs[0].rowid;
                        if !intent.keywords.is_empty() {
                            'outer: for msg_item in &msgs {
                                if let Some(ref mc) = mail {
                                    let mc2 = mc.clone();
                                    let rid = msg_item.rowid;
                                    if let Some(full) =
                                        tokio::task::spawn_blocking(move || mc2.get_message(rid))
                                            .await
                                            .ok()
                                            .and_then(|r| r.ok())
                                            .flatten()
                                    {
                                        if let Some(ref body) = full.body {
                                            let body_low = body.to_lowercase();
                                            if intent
                                                .keywords
                                                .iter()
                                                .any(|kw| body_low.contains(kw.as_str()))
                                            {
                                                best_rowid = msg_item.rowid;
                                                break 'outer;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        let found =
                            enrich_message(best_rowid, &mail, is_attachment, &mut lines).await;
                        pdf_paths.extend(found);

                        let best_msg = msgs
                            .iter()
                            .find(|m| m.rowid == best_rowid)
                            .unwrap_or(&msgs[0]);
                        found_mail_ref = Some(MailRef {
                            rowid: best_rowid,
                            message_id: None,
                            subject: best_msg.subject.clone(),
                            sender: best_msg.sender.clone(),
                            auto_open: wants_open,
                        });

                        parts.push(format!(
                            "Nájdené emaily (Apple Mail):\n{}",
                            lines.join("\n")
                        ));
                    }
                } // end if !already_handled
            }

            _ => {
                let msgs = fetch_recent_mails(&db, &mail, 10).await;
                if !msgs.is_empty() {
                    let lines: Vec<String> = msgs
                        .iter()
                        .map(|m| {
                            let from = m.sender_display.as_deref().unwrap_or(&m.sender);
                            let status = if m.is_read { "✓" } else { "●" };
                            format!(
                                "  {status} [{}] Od: {} | Predmet: {}",
                                relative_date(m.received_at),
                                from,
                                m.subject
                            )
                        })
                        .collect();
                    parts.push(format!(
                        "Posledné emaily (Apple Mail):\n{}",
                        lines.join("\n")
                    ));
                }
            }
        }
    }

    // ── Notes ─────────────────────────────────────────────────────────────────
    if wants_notes {
        if let Some(n) = notes {
            let nc = n.clone();
            if let Ok(Ok(items)) = tokio::task::spawn_blocking(move || nc.list_notes(5)).await {
                if !items.is_empty() {
                    let lines: Vec<String> = items
                        .iter()
                        .map(|note| {
                            let folder = note.folder.as_deref().unwrap_or("Notes");
                            let snip = note.snippet.as_deref().unwrap_or("");
                            format!("  [{}] {} — {}", folder, note.title, snip)
                        })
                        .collect();
                    parts.push(format!(
                        "Posledné poznámky (Apple Notes):\n{}",
                        lines.join("\n")
                    ));
                }
            }
        }
    }

    // ── AeroSpace window management ───────────────────────────────────────────
    // Cheap keyword gate — only invoke the classifier when the turn looks
    // like a window/workspace management request.
    let looks_like_window = [
        "plochu",
        "ploch",
        "workspace",
        "prepni",
        "presuň",
        "presun",
        "zameraj",
        "otvor na ploch",
    ]
    .iter()
    .any(|kw| low.contains(kw));

    if looks_like_window {
        if let Ok(intent) = WindowIntentClassifier::new(ollama.clone(), model.clone())
            .classify(message, &history_snippet)
            .await
        {
            tracing::debug!("window_intent: {:?}", intent);
            if intent.action != "none" {
                if let Ok(note) = run_aerospace_intent(&intent).await {
                    // Background action completed — signal caller to skip LLM streaming
                    // and show a brief confirmation in the voice notch instead.
                    tracing::info!("window action taken: {}", note);
                    return (None, Vec::new(), None, Some(note), None);
                }
            }
        }
    }

    // ── Odoo ──────────────────────────────────────────────────────────────────
    {
        let odoo_guard = odoo.read().await;
        let odoo_configured = odoo_guard.is_some();
        drop(odoo_guard);

        // Keyword gate — independent of the context_planner's is_odoo(), because
        // "faktúra" routes to invoice_analysis there (→ hits this branch too).
        let looks_like_odoo = odoo_configured
            && ([
                "odoo",
                "faktúr",
                "faktura",
                "invoice",
                "partner",
                "kontakt",
                "zákazník",
                "zakaznik",
                "helpdesk",
                "tiket",
                "ticket",
                "úloh",
                "uloh",
                "objednávk",
            ]
            .iter()
            .any(|kw| low.contains(kw))
                || (last_odoo_ref.is_some()
                    && [
                        "otvor to",
                        "otvor ho",
                        "otvor ju",
                        "open it",
                        "otvoriť",
                        "ten záznam",
                        "that record",
                        "show in odoo",
                    ]
                    .iter()
                    .any(|kw| low.contains(kw))));

        if looks_like_odoo {
            let odoo_guard = odoo.read().await;
            if let Some(ref conn) = *odoo_guard {
                let intent = OdooIntentClassifier::new(ollama.clone(), model.clone())
                    .classify(message, &history_snippet)
                    .await
                    .unwrap_or_default();

                tracing::debug!("odoo_intent: {:?}", intent);

                match intent.action {
                    OdooAction::SearchContacts => {
                        let query = intent.query.as_deref().unwrap_or("");
                        match conn.search_partners(query, 5).await {
                            Ok(partners) if !partners.is_empty() => {
                                let lines: Vec<String> = partners
                                    .iter()
                                    .map(|p| {
                                        let mut line = format!(
                                            "  • {} (id:{})",
                                            p.name.as_deref().unwrap_or("—"),
                                            p.id
                                        );
                                        if let Some(e) = &p.email {
                                            line.push_str(&format!(", email: {e}"));
                                        }
                                        if let Some(ph) = &p.phone {
                                            line.push_str(&format!(", tel: {ph}"));
                                        }
                                        if let Some(v) = &p.vat {
                                            line.push_str(&format!(", IČO/DIČ: {v}"));
                                        }
                                        if let Some(c) = &p.city {
                                            line.push_str(&format!(", {c}"));
                                        }
                                        line
                                    })
                                    .collect();
                                parts.push(format!(
                                    "## Živé dáta z Odoo — partneri\n{}",
                                    lines.join("\n")
                                ));
                                if let Some(p) = partners.first() {
                                    let name = p.name.as_deref().unwrap_or("Partner");
                                    found_odoo_ref =
                                        Some(conn.record_ref("res.partner", p.id, name));
                                }
                            }
                            Ok(_) => parts
                                .push(format!("Odoo: žiadny partner nenájdený pre \"{query}\".")),
                            Err(e) => {
                                tracing::warn!("Odoo search_partners error: {e}");
                                parts.push("Odoo partner vyhľadávanie nedostupné.".into());
                            }
                        }
                    }

                    OdooAction::GetInvoices => {
                        match conn.my_invoices(intent.open_only, 10).await {
                            Ok(invoices) if !invoices.is_empty() => {
                                let lines: Vec<String> = invoices.iter().map(|inv| {
                                    let num = inv.name.as_deref().unwrap_or("—");
                                    let partner = inv.partner_id.display();
                                    let amt = inv.amount_total.map(|a| format!("{a:.2}")).unwrap_or_else(|| "—".into());
                                    let cur = inv.currency_id.display();
                                    let state = inv.payment_state.as_deref().unwrap_or("—");
                                    let date = inv.invoice_date.as_deref().unwrap_or("—");
                                    format!("  • {num} | {partner} | {amt} {cur} | {state} | {date}")
                                }).collect();
                                let header = if intent.open_only {
                                    "neuhradené faktúry"
                                } else {
                                    "faktúry"
                                };
                                parts.push(format!(
                                    "## Živé dáta z Odoo — {header}\n{}",
                                    lines.join("\n")
                                ));
                                if let Some(inv) = invoices.first() {
                                    let name = inv.name.as_deref().unwrap_or("Faktúra");
                                    found_odoo_ref =
                                        Some(conn.record_ref("account.move", inv.id, name));
                                }
                            }
                            Ok(_) => parts.push("Odoo: žiadne faktúry nenájdené.".into()),
                            Err(e) => {
                                tracing::warn!("Odoo my_invoices error: {e}");
                                parts.push("Odoo faktúry nedostupné.".into());
                            }
                        }
                    }

                    OdooAction::ListTickets => {
                        match conn.my_helpdesk_tickets(intent.open_only, 10).await {
                            Ok(tickets) if !tickets.is_empty() => {
                                let lines: Vec<String> = tickets
                                    .iter()
                                    .map(|t| {
                                        let name = t.name.as_deref().unwrap_or("—");
                                        let stage = t.stage_id.display();
                                        let partner = t.partner_id.display();
                                        let date = t.create_date.as_deref().unwrap_or("—");
                                        format!("  • [{stage}] {name} | {partner} | {date}")
                                    })
                                    .collect();
                                let header = if intent.open_only {
                                    "otvorené helpdesk tikety"
                                } else {
                                    "helpdesk tikety"
                                };
                                parts.push(format!(
                                    "## Živé dáta z Odoo — {header}\n{}",
                                    lines.join("\n")
                                ));
                                if let Some(t) = tickets.first() {
                                    let name = t.name.as_deref().unwrap_or("Tiket");
                                    found_odoo_ref =
                                        Some(conn.record_ref("helpdesk.ticket", t.id, name));
                                }
                            }
                            Ok(_) => parts.push("Odoo: žiadne helpdesk tikety nenájdené.".into()),
                            Err(e) => {
                                tracing::warn!("Odoo my_helpdesk_tickets error: {e}");
                                parts.push("Odoo helpdesk nedostupný.".into());
                            }
                        }
                    }

                    OdooAction::GetRecord => {
                        if let (Some(model_str), Some(id)) =
                            (intent.query.as_deref(), intent.record_id)
                        {
                            match conn.get_record(model_str, id).await {
                                Ok(Some(rec)) => {
                                    let name = rec["name"]
                                        .as_str()
                                        .or_else(|| rec["display_name"].as_str())
                                        .unwrap_or("Záznam")
                                        .to_string();
                                    parts.push(format!("## Živé dáta z Odoo — záznam\n{rec}"));
                                    found_odoo_ref = Some(conn.record_ref(model_str, id, &name));
                                }
                                Ok(None) => {
                                    parts.push(format!("Odoo: záznam {model_str} #{id} nenájdený."))
                                }
                                Err(e) => {
                                    tracing::warn!("Odoo get_record error: {e}");
                                    parts.push("Odoo záznam nedostupný.".into());
                                }
                            }
                        }
                    }

                    OdooAction::Open => {
                        // Resolve the record to open: from intent or last_odoo_ref.
                        let target_ref = last_odoo_ref.cloned().or_else(|| found_odoo_ref.clone());
                        if let Some(ref r) = target_ref {
                            let url = r.url.clone();
                            tokio::spawn(async move {
                                if let Err(e) = fs_open::open_url_in_safari(&url).await {
                                    tracing::warn!("odoo open_url_in_safari error: {e}");
                                }
                            });
                            found_odoo_ref = target_ref;
                        } else {
                            parts.push(
                                "Odoo: žiadny záznam na otvorenie. Najprv vyhľadajte záznam."
                                    .into(),
                            );
                        }
                    }

                    OdooAction::None => {}
                }
            }
        }
    }

    // ── WhatsApp (Phase 11) ───────────────────────────────────────────────────
    {
        // Keyword gate — only run classifier when turn looks WhatsApp-related.
        let looks_like_whatsapp = {
            let explicit = ["whatsapp", " wa ", "na whatsappe", "cez whatsapp"];
            let send_signals = [
                "napíš mu",
                "napíš jej",
                "pošli mu správu",
                "pošli jej správu",
                "napíš petrovi",
                "napíš katke",
                "write to ",
                "send to ",
            ];
            let find_signals = [
                "kde mi písal",
                "kde mi písala",
                "čo mi písal",
                "čo mi písala",
                "čo sme si písali",
                "nájdi správu od",
                "find message from",
            ];
            let has_mail = low.contains("mail") || low.contains("email");
            explicit.iter().any(|k| low.contains(k))
                || (send_signals.iter().any(|k| low.contains(k)) && !has_mail)
                || (find_signals.iter().any(|k| low.contains(k)) && !has_mail)
        };

        if looks_like_whatsapp {
            // Check bridge accessibility (cheap — no HTTP call when not started).
            let wa_status = whatsapp.status().await;
            let is_ready = matches!(
                wa_status.as_ref().map(|s| &s.status),
                Ok(WhatsappConnectionStatus::Ready)
            );
            let is_qr = matches!(
                wa_status.as_ref().map(|s| &s.status),
                Ok(WhatsappConnectionStatus::Qr)
            );
            let is_missing_node = matches!(
                wa_status.as_ref().map(|s| &s.status),
                Ok(WhatsappConnectionStatus::MissingNode)
            );
            let is_not_installed = matches!(
                wa_status.as_ref().map(|s| &s.status),
                Ok(WhatsappConnectionStatus::BridgeNotInstalled)
            );

            if is_qr {
                parts.push(
                    "## WhatsApp\nWhatsApp čaká na QR kód — prejdi do Nastavení a naskenuj QR."
                        .into(),
                );
            } else if is_missing_node {
                parts.push(
                    "## WhatsApp\nNode.js nie je nainštalovaný — WhatsApp bridge nedostupný."
                        .into(),
                );
            } else if is_not_installed {
                parts.push("## WhatsApp\nWhatsApp bridge nie je nainštalovaný — spusti `make whatsapp-bridge-install`.".into());
            } else if !is_ready {
                parts.push(
                    "## WhatsApp\nWhatsApp nie je pripojený — v Nastaveniach sa pripojiť.".into(),
                );
            } else {
                // Classify intent
                let wa_intent = WhatsappIntentClassifier::new(ollama.clone(), model.clone())
                    .classify(message, &history_snippet)
                    .await
                    .unwrap_or_default();

                tracing::debug!("whatsapp_intent: {:?}", wa_intent);

                match wa_intent.action {
                    WhatsappAction::ListRecent => match whatsapp.list_chats(10).await {
                        Ok(chats) if !chats.is_empty() => {
                            let lines: Vec<String> = chats
                                .iter()
                                .map(|c| {
                                    let name = c.name.as_deref().unwrap_or("—");
                                    let unread = if c.unread_count > 0 {
                                        format!(" [{}]", c.unread_count)
                                    } else {
                                        String::new()
                                    };
                                    let preview = c
                                        .last_message_preview
                                        .as_deref()
                                        .unwrap_or("")
                                        .chars()
                                        .take(60)
                                        .collect::<String>();
                                    format!("  • {name}{unread}: {preview}")
                                })
                                .collect();
                            parts.push(format!(
                                "## WhatsApp — posledné chaty (PII)\n{}",
                                lines.join("\n")
                            ));
                        }
                        Ok(_) => parts.push("WhatsApp: žiadne chaty nenájdené.".into()),
                        Err(e) => {
                            tracing::warn!("WhatsApp list_chats error: {e}");
                            parts.push("WhatsApp chaty nedostupné.".into());
                        }
                    },

                    WhatsappAction::Search => {
                        // Search cached messages (LIKE + recency)
                        let keywords = if wa_intent.keywords.is_empty() {
                            message
                                .split_whitespace()
                                .filter(|w| w.len() > 3)
                                .take(5)
                                .map(|s| s.to_string())
                                .collect::<Vec<_>>()
                        } else {
                            wa_intent.keywords.clone()
                        };
                        let results = search_whatsapp_cache(&db, &keywords, 8).await;
                        if !results.is_empty() {
                            let lines: Vec<String> = results
                                .iter()
                                .map(|r| {
                                    let body_preview = r.body.chars().take(300).collect::<String>();
                                    format!(
                                        "  [{sender}] {body}",
                                        sender = r.sender,
                                        body = body_preview
                                    )
                                })
                                .collect();
                            parts.push(format!(
                                "## WhatsApp — hľadanie (PII)\n{}",
                                lines.join("\n")
                            ));
                        } else {
                            // Fallback: live bridge recent chats
                            match whatsapp.list_chats(5).await {
                                Ok(chats) if !chats.is_empty() => {
                                    let lines: Vec<String> = chats
                                        .iter()
                                        .filter_map(|c| {
                                            c.last_message_preview.as_deref().map(|p| {
                                                let name = c.name.as_deref().unwrap_or("—");
                                                format!(
                                                    "  • {name}: {}",
                                                    p.chars().take(80).collect::<String>()
                                                )
                                            })
                                        })
                                        .collect();
                                    if !lines.is_empty() {
                                        parts.push(format!(
                                            "## WhatsApp — posledné správy (PII)\n{}",
                                            lines.join("\n")
                                        ));
                                    } else {
                                        parts.push(format!(
                                            "WhatsApp: žiadne správy nenájdené pre: {}.",
                                            keywords.join(", ")
                                        ));
                                    }
                                }
                                _ => {
                                    parts.push(format!(
                                        "WhatsApp: žiadne správy nenájdené pre: {}.",
                                        keywords.join(", ")
                                    ));
                                }
                            }
                        }
                    }

                    WhatsappAction::ReadHistory => {
                        // Try to resolve contact → chat
                        let contact_query = wa_intent
                            .contact_name
                            .as_deref()
                            .or(wa_intent.phone.as_deref())
                            .unwrap_or("");
                        if !contact_query.is_empty() {
                            // Search chats for name match
                            match whatsapp.list_chats(50).await {
                                Ok(chats) => {
                                    let matched = chats.iter().find(|c| {
                                        c.name
                                            .as_deref()
                                            .map(|n| {
                                                n.to_lowercase()
                                                    .contains(&contact_query.to_lowercase())
                                            })
                                            .unwrap_or(false)
                                    });
                                    if let Some(chat) = matched {
                                        match whatsapp.chat_messages(&chat.id, 10, None).await {
                                            Ok(msgs) if !msgs.is_empty() => {
                                                let lines: Vec<String> = msgs
                                                    .iter()
                                                    .rev()
                                                    .take(8)
                                                    .map(|m| {
                                                        let sender = if m.from_me {
                                                            "Ja"
                                                        } else {
                                                            chat.name.as_deref().unwrap_or(&m.from)
                                                        };
                                                        let body = m
                                                            .body
                                                            .chars()
                                                            .take(300)
                                                            .collect::<String>();
                                                        format!("  [{sender}]: {body}")
                                                    })
                                                    .collect();
                                                let chat_name =
                                                    chat.name.as_deref().unwrap_or(contact_query);
                                                parts.push(format!("## WhatsApp — história s {chat_name} (PII)\n{}", lines.join("\n")));

                                                // Cache messages in DB for future searches
                                                let msgs_to_cache = msgs.clone();
                                                let db_cache = db.clone();
                                                let chat_name_str =
                                                    chat.name.clone().unwrap_or_default();
                                                tokio::spawn(async move {
                                                    cache_whatsapp_messages(
                                                        &db_cache,
                                                        &msgs_to_cache,
                                                        &chat_name_str,
                                                    )
                                                    .await;
                                                });
                                            }
                                            Ok(_) => parts.push(format!(
                                                "WhatsApp: žiadne správy s {}.",
                                                contact_query
                                            )),
                                            Err(e) => {
                                                tracing::warn!("WhatsApp chat_messages error: {e}");
                                                parts.push("WhatsApp história nedostupná.".into());
                                            }
                                        }
                                    } else {
                                        parts.push(format!(
                                            "WhatsApp: kontakt \"{}\" nenájdený.",
                                            contact_query
                                        ));
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("WhatsApp list_chats error: {e}");
                                    parts.push("WhatsApp chaty nedostupné.".into());
                                }
                            }
                        } else {
                            parts.push("WhatsApp: uveď meno kontaktu.".into());
                        }
                    }

                    WhatsappAction::DraftSend => {
                        let contact = wa_intent
                            .contact_name
                            .as_deref()
                            .or(wa_intent.phone.as_deref());
                        let text = wa_intent.message_text.as_deref().unwrap_or("");

                        if text.is_empty() {
                            parts.push("WhatsApp: text správy chýba.".into());
                        } else if contact.is_none() {
                            parts.push("WhatsApp: príjemca správy chýba.".into());
                        } else {
                            let contact_str = contact.unwrap().to_string();
                            let text_str = text.to_string();

                            // Resolve chat JID from contact name
                            let chat_id_opt = match whatsapp.list_chats(50).await {
                                Ok(chats) => chats
                                    .iter()
                                    .find(|c| {
                                        c.name
                                            .as_deref()
                                            .map(|n| {
                                                n.to_lowercase()
                                                    .contains(&contact_str.to_lowercase())
                                            })
                                            .unwrap_or(false)
                                    })
                                    .map(|c| c.id.clone()),
                                Err(_) => None,
                            };

                            let target_opt: Option<WhatsappSendTarget> = match chat_id_opt {
                                Some(id) => Some(WhatsappSendTarget::ChatId(id)),
                                None => wa_intent.phone.clone().map(WhatsappSendTarget::Phone),
                            };
                            if target_opt.is_none() {
                                parts.push(format!(
                                    "WhatsApp: kontakt \"{}\" nenájdený. Uveď telefónne číslo.",
                                    contact_str
                                ));
                            }
                            if let Some(target) = target_opt {
                                let recipient_display = &contact_str;
                                // Check rules gate — Forbidden blocks immediately
                                let rules_level = rules.check("whatsapp.send_message", "{}");
                                if matches!(rules_level, ApprovalLevel::Forbidden) {
                                    parts.push(
                                        "WhatsApp: posielanie správ je zakázané pravidlami.".into(),
                                    );
                                } else {
                                    // Approval flow (trap #1: always request regardless of Auto/Ask)
                                    let text_preview = if text_str.len() > 60 {
                                        format!("{}… ({} znakov)", &text_str[..60], text_str.len())
                                    } else {
                                        text_str.clone()
                                    };
                                    let audit_description = format!(
                                        "WhatsApp správa — Príjemca: {} | Náhľad: {}",
                                        recipient_display, text_preview
                                    );

                                    // We don't have a tx sender here (called outside SSE stream context
                                    // in the REST-style tool-context path). Use None → Swift polls /approvals/pending.
                                    let approved = request_approval_core(
                                        &db,
                                        &pending_approvals,
                                        "whatsapp.send_message",
                                        &audit_description,
                                        None,
                                    )
                                    .await;

                                    if !approved {
                                        parts.push(format!(
                                        "WhatsApp: odoslanie správy pre \"{}\" bolo zamietnuté.",
                                        recipient_display
                                    ));
                                    } else {
                                        match whatsapp.send_message(target, &text_str).await {
                                            Ok(msg_ref) => {
                                                tracing::info!(message_id = %msg_ref.message_id, "WhatsApp message sent via tool context");
                                                parts.push(format!(
                                                "WhatsApp: správa pre \"{}\" bola odoslaná (id: {}).",
                                                recipient_display, msg_ref.message_id
                                            ));
                                            }
                                            Err(e) => {
                                                tracing::warn!(
                                                    "WhatsApp send error in tool ctx: {e}"
                                                );
                                                parts.push(format!(
                                                    "WhatsApp: odoslanie zlyhalo — {}",
                                                    e
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } // end DraftSend

                    WhatsappAction::None => {}
                }
            }
        }
    }

    let ctx = if parts.is_empty() {
        None
    } else {
        Some(format!(
            "## Živé dáta z tvojich aplikácií\n\
             Ak sú nájdené dáta, odpovedaj z nich priamo a presne.\n\
             Ak email alebo obsah nie je nájdený, povedz to používateľovi jasne.\n\n{}",
            parts.join("\n\n")
        ))
    };

    (ctx, pdf_paths, found_mail_ref, None, found_odoo_ref)
}

// ── WhatsApp DB helpers ───────────────────────────────────────────────────────

/// Simple row returned by `search_whatsapp_cache`.
struct WaCacheRow {
    sender: String,
    body: String,
}

/// Search the messages cache for WhatsApp messages matching `keywords` via LIKE.
/// Returns up to `limit` rows ordered by recency (newest first).
async fn search_whatsapp_cache(
    db: &Arc<Mutex<Connection>>,
    keywords: &[String],
    limit: usize,
) -> Vec<WaCacheRow> {
    if keywords.is_empty() {
        return vec![];
    }
    let Ok(db) = db.try_lock() else {
        return vec![];
    };

    // Build a LIKE clause for each keyword: body LIKE '%kw%'
    let clauses: Vec<String> = keywords
        .iter()
        .map(|k| format!("body LIKE '%{}%'", k.replace('\'', "''")))
        .collect();
    let where_clause = clauses.join(" OR ");
    let sql = format!(
        "SELECT sender, body FROM messages \
         WHERE source = 'whatsapp' AND ({where_clause}) \
         ORDER BY received_at DESC LIMIT {limit}"
    );

    let mut stmt = match db.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("search_whatsapp_cache prepare error: {e}");
            return vec![];
        }
    };
    stmt.query_map([], |row| {
        Ok(WaCacheRow {
            sender: row.get::<_, String>(0).unwrap_or_default(),
            body: row.get::<_, String>(1).unwrap_or_default(),
        })
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// Upsert WhatsApp messages into the messages table (idempotent on external_id).
/// chat_name goes into `subject`; `metadata_json` stores chat_id / from_me / has_media.
async fn cache_whatsapp_messages(
    db: &Arc<Mutex<Connection>>,
    msgs: &[whatsapp_connector::WhatsappMessage],
    chat_name: &str,
) {
    let Ok(db) = db.try_lock() else {
        return;
    };
    for msg in msgs {
        let meta = serde_json::json!({
            "chat_id": msg.chat_id,
            "from_me": msg.from_me,
            "has_media": msg.has_media,
            "to": msg.to,
        });
        let _ = db.execute(
            "INSERT OR IGNORE INTO messages \
             (source, external_id, subject, body, sender, received_at, metadata_json) \
             VALUES ('whatsapp', ?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                msg.id,
                chat_name,
                msg.body,
                msg.from,
                msg.timestamp,
                meta.to_string(),
            ],
        );
    }
}

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

/// Execute a `WindowIntent` using the aerospace CLI.
/// Returns a short SK confirmation string to inject into the LLM context,
/// or an empty string if the binary is absent (silent degrade).
async fn run_aerospace_intent(intent: &bagent_agent::WindowIntent) -> anyhow::Result<String> {
    match intent.action.as_str() {
        "focus_workspace" => {
            if let Some(ref ws) = intent.workspace {
                match run_aerospace(&["workspace", ws]).await {
                    Ok(_) => return Ok(format!("Prepnuté na plochu {ws}.")),
                    Err(e) => {
                        tracing::warn!("aerospace focus_workspace: {e}");
                        return Ok(String::new());
                    }
                }
            }
        }

        "open_app" => {
            let app = intent.app.as_deref().unwrap_or("Mail");
            // 1. Launch (or focus) the application
            let _ = tokio::process::Command::new("open")
                .args(["-a", app])
                .output()
                .await;

            if let Some(ref ws) = intent.workspace {
                // 2. Poll until the window appears, then move it
                let bundle_id = app_to_bundle_id(app);
                let ws_str = ws.clone();
                let app_str = app.to_string();
                tokio::spawn(async move {
                    for _ in 0..30 {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        let Ok(list) = run_aerospace(&[
                            "list-windows",
                            "--all",
                            "--app-bundle-id",
                            &bundle_id,
                            "--format",
                            "%{window-id}",
                        ])
                        .await
                        else {
                            continue;
                        };
                        let wid = list.lines().next().unwrap_or("").trim().to_string();
                        if wid.is_empty() {
                            continue;
                        }
                        // Try with --window-id first (AeroSpace 0.15+), fall back to focus+move
                        let moved = run_aerospace(&[
                            "move-node-to-workspace",
                            "--window-id",
                            &wid,
                            &ws_str,
                        ])
                        .await;
                        if moved.is_err() {
                            let _ = run_aerospace(&["focus", "--window-id", &wid]).await;
                            let _ = run_aerospace(&["move-node-to-workspace", &ws_str]).await;
                        }
                        tracing::info!(
                            "aerospace: moved {app_str} (wid={wid}) to workspace {ws_str}"
                        );
                        break;
                    }
                });
                return Ok(format!("Otváram {app} a presúvam na plochu {ws}."));
            } else {
                return Ok(format!("Otváram {app}."));
            }
        }

        "move_app" => {
            let app = intent.app.as_deref().unwrap_or("Mail");
            let ws = intent.workspace.as_deref().unwrap_or("1");
            let bundle_id = app_to_bundle_id(app);
            let Ok(list) = run_aerospace(&[
                "list-windows",
                "--all",
                "--app-bundle-id",
                &bundle_id,
                "--format",
                "%{window-id}",
            ])
            .await
            else {
                tracing::warn!("aerospace move_app: could not list windows");
                return Ok(String::new());
            };
            let wid = list.lines().next().unwrap_or("").trim().to_string();
            if !wid.is_empty() {
                let moved =
                    run_aerospace(&["move-node-to-workspace", "--window-id", &wid, ws]).await;
                if moved.is_err() {
                    let _ = run_aerospace(&["focus", "--window-id", &wid]).await;
                    let _ = run_aerospace(&["move-node-to-workspace", ws]).await;
                }
                return Ok(format!("Okno {app} presunuté na plochu {ws}."));
            }
        }

        "focus_app" => {
            let app = intent.app.as_deref().unwrap_or("Mail");
            let bundle_id = app_to_bundle_id(app);
            let Ok(list) = run_aerospace(&[
                "list-windows",
                "--all",
                "--app-bundle-id",
                &bundle_id,
                "--format",
                "%{window-id}",
            ])
            .await
            else {
                return Ok(String::new());
            };
            let wid = list.lines().next().unwrap_or("").trim().to_string();
            if !wid.is_empty() {
                let _ = run_aerospace(&["focus", "--window-id", &wid]).await;
                return Ok(format!("Zameraný na {app}."));
            }
        }

        _ => {}
    }
    Ok(String::new())
}

/// Map common app display names to their bundle IDs for `aerospace list-windows`.
fn app_to_bundle_id(app: &str) -> String {
    match app.to_lowercase().as_str() {
        "mail" => "com.apple.mail".to_string(),
        "safari" => "com.apple.Safari".to_string(),
        "notes" => "com.apple.Notes".to_string(),
        "finder" => "com.apple.finder".to_string(),
        "terminal" => "com.apple.Terminal".to_string(),
        "xcode" => "com.apple.dt.Xcode".to_string(),
        "vscode" | "visual studio code" => "com.microsoft.VSCode".to_string(),
        "slack" => "com.tinyspeck.slackmacgap".to_string(),
        _ => format!("com.apple.{}", app.to_lowercase()),
    }
}

fn relative_date(unix: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let diff = now.saturating_sub(unix);
    match diff / 3600 {
        0 => "práve teraz".to_string(),
        1 => "pred 1 hodinou".to_string(),
        2..=23 => format!("pred {} hodinami", diff / 3600),
        24..=47 => "včera".to_string(),
        hours => format!("pred {} dňami", hours / 24),
    }
}

// ── Context management ────────────────────────────────────────────────────────

async fn load_session_summary(db: &Arc<Mutex<Connection>>, session_id: &str) -> Option<String> {
    db.lock()
        .await
        .query_row(
            "SELECT summary FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
}

/// Read-modify-write a single key inside `sessions.metadata_json`.
/// Safe to call concurrently: each call holds the mutex for the duration.
async fn merge_session_metadata(
    db: &Arc<Mutex<Connection>>,
    session_id: &str,
    key: &str,
    value: serde_json::Value,
) {
    let existing: Option<String> = db
        .lock()
        .await
        .query_row(
            "SELECT metadata_json FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();

    let mut blob: serde_json::Value = existing
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::Value::Object(Default::default()));

    if let Some(obj) = blob.as_object_mut() {
        obj.insert(key.to_string(), value);
    }

    if let Ok(json) = serde_json::to_string(&blob) {
        let _ = db.lock().await.execute(
            "UPDATE sessions SET metadata_json = ?1 WHERE id = ?2",
            rusqlite::params![json, session_id],
        );
    }
}

async fn save_last_mail_ref(db: &Arc<Mutex<Connection>>, session_id: &str, mail_ref: &MailRef) {
    let val = serde_json::to_value(mail_ref).unwrap_or(serde_json::Value::Null);
    merge_session_metadata(db, session_id, "last_mail_ref", val).await;
}

async fn load_last_mail_ref(db: &Arc<Mutex<Connection>>, session_id: &str) -> Option<MailRef> {
    let json: Option<String> = db
        .lock()
        .await
        .query_row(
            "SELECT metadata_json FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let val: serde_json::Value = serde_json::from_str(&json?).ok()?;
    serde_json::from_value(val["last_mail_ref"].clone()).ok()
}

async fn save_last_file_ref(db: &Arc<Mutex<Connection>>, session_id: &str, file_ref: &FileRef) {
    let val = serde_json::to_value(file_ref).unwrap_or(serde_json::Value::Null);
    merge_session_metadata(db, session_id, "last_file_ref", val).await;
}

async fn load_last_file_ref(db: &Arc<Mutex<Connection>>, session_id: &str) -> Option<FileRef> {
    let json: Option<String> = db
        .lock()
        .await
        .query_row(
            "SELECT metadata_json FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let val: serde_json::Value = serde_json::from_str(&json?).ok()?;
    serde_json::from_value(val["last_file_ref"].clone()).ok()
}

async fn save_last_odoo_ref(
    db: &Arc<Mutex<Connection>>,
    session_id: &str,
    odoo_ref: &OdooRecordRef,
) {
    let val = serde_json::to_value(odoo_ref).unwrap_or(serde_json::Value::Null);
    merge_session_metadata(db, session_id, "last_odoo_ref", val).await;
}

async fn load_last_odoo_ref(
    db: &Arc<Mutex<Connection>>,
    session_id: &str,
) -> Option<OdooRecordRef> {
    let json: Option<String> = db
        .lock()
        .await
        .query_row(
            "SELECT metadata_json FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let val: serde_json::Value = serde_json::from_str(&json?).ok()?;
    serde_json::from_value(val["last_odoo_ref"].clone()).ok()
}

async fn load_session_history(db: &Arc<Mutex<Connection>>, session_id: &str) -> Vec<Message> {
    let db = db.lock().await;
    let mut stmt = match db.prepare(
        "SELECT role, content FROM chat_turns \
         WHERE session_id = ?1 ORDER BY created_at DESC LIMIT 10",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let mut turns: Vec<Message> = stmt
        .query_map(rusqlite::params![session_id], |row| {
            let role: String = row.get(0)?;
            let content: String = row.get(1)?;
            Ok(match role.as_str() {
                "user" => Message::user(content),
                "assistant" => Message::assistant(content),
                _ => Message::system(content),
            })
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();
    turns.reverse();
    turns
}

async fn prepare_history(
    ollama: &OllamaClient,
    model: &str,
    history: Vec<OllamaMsg>,
) -> Vec<OllamaMsg> {
    if history.len() > SUMMARIZE_THRESHOLD {
        let split = history.len() - KEEP_RECENT;
        let old = &history[..split];
        let recent = history[split..].to_vec();

        if let Ok(summary) = ollama.summarize(model, old).await {
            let mut result = vec![Message::system(format!(
                "Zhrnutie predchádzajúcej konverzácie: {summary}"
            ))];
            result.extend(recent);
            return result;
        }
        history[history.len() - MAX_HISTORY..].to_vec()
    } else if history.len() > MAX_HISTORY {
        history[history.len() - MAX_HISTORY..].to_vec()
    } else {
        history
    }
}

fn err_event(msg: &str) -> Event {
    Event::default().data(serde_json::json!({"type":"error","message":msg}).to_string())
}
