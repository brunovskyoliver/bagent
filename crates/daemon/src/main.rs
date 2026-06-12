use axum::{
    Router,
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{delete, get, post},
    Json,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use sha2::{Digest, Sha256};
use bagent_agent::{
    CorrectionClassifier, DirectiveExtractor, MailIntentClassifier, MemoryExtractor,
    PromptBuilder, WindowIntentClassifier,
    has_explicit_trigger,
};
use apple_mail_connector::MailSearchFilter;
use bagent_attachments::extract as extract_attachment;
use bagent_memory::MemoryStore;
use bagent_rules::{ApprovalLevel, RuleEngine, DEFAULT_RULES_YAML};
use futures_util::StreamExt;
use ollama_connector::{Message, OllamaClient, DEFAULT_BASE_URL, DEFAULT_EMBED_MODEL};
use apple_mail_connector::{self, MailConnector};
use apple_notes_connector::NotesConnector;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, convert::Infallible, path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use anyhow::Result;

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
    /// Small fast model for intent/correction classifiers — never blocks chat TTFT.
    classifier_model: String,
    vision_model: String,
    attachments_dir: PathBuf,
    ollama: OllamaClient,
    mail: Option<MailConnector>,
    notes: Option<NotesConnector>,
    memory: Arc<MemoryStore>,
    prompt_builder: Arc<PromptBuilder>,
    rules: Arc<RuleEngine>,
    pending_approvals: Arc<std::sync::Mutex<HashMap<String, oneshot::Sender<bool>>>>,
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
}

fn default_und() -> String { "und".to_string() }

#[derive(Deserialize)]
struct MemorySearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    namespace: String,
    #[serde(default = "default_limit")]
    limit: usize,
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

fn default_limit() -> usize { 20 }

/// Stable reference to a found mail message — surfaced to the frontend so
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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let data_dir = app_data_dir();
    std::fs::create_dir_all(&data_dir)?;
    let attachments_dir = data_dir.join("attachments");
    std::fs::create_dir_all(&attachments_dir)?;

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

    if mail.is_some()  { tracing::info!("Mail connector: accessible"); }
    else               { tracing::warn!("Mail connector: no Full Disk Access"); }
    if notes.is_some() { tracing::info!("Notes connector: accessible"); }
    else               { tracing::warn!("Notes connector: no Full Disk Access"); }

    let ollama = OllamaClient::new(DEFAULT_BASE_URL);

    // MemoryStore uses a separate connection with std::sync::Mutex (blocking SQLite ops)
    let mem_conn = rusqlite::Connection::open(data_dir.join("bagent.db"))?;
    let mem_db = Arc::new(std::sync::Mutex::new(mem_conn));
    let memory = Arc::new(MemoryStore::new(mem_db, ollama.clone()));
    let prompt_builder = Arc::new(PromptBuilder::new(memory.clone()));

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
            if r_chat.is_ok()  { tracing::info!("warmup: chat model loaded"); }
            if r_embed.is_ok() { tracing::info!("warmup: embed model loaded"); }
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
                Err(e) => { tracing::warn!("backfill: failed to open db: {e}"); return; }
            };
            let ids: Vec<(String, String)> = {
                let mut stmt = match conn.prepare(
                    "SELECT id, content FROM chat_turns \
                     WHERE id NOT IN (SELECT item_id FROM embeddings WHERE source='chat_turn') \
                     AND role IN ('user','assistant') \
                     LIMIT 200"
                ) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                stmt.query_map([], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?)))
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
                match mail_sync_inner(db_poll.clone(), mail_for_poll.clone(), memory_poll.clone()).await {
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
                match mail_sync_inner(db_poll.clone(), mail_for_poll.clone(), memory_poll.clone()).await {
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
                    if watcher.watch(&mail_wal, RecursiveMode::NonRecursive).is_ok() {
                        tokio::spawn(async move {
                            let _watcher = watcher; // keep alive
                            loop {
                                if tok_rx.recv().await.is_none() { break; }
                                // Debounce: drain any burst events
                                while tok_rx.try_recv().is_ok() {}
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                while tok_rx.try_recv().is_ok() {}
                                if is_on_ac_power() {
                                    tracing::info!("mail FSEvents: WAL changed, syncing");
                                    match mail_sync_inner(db_fs.clone(), mail_for_fs.clone(), memory_fs.clone()).await {
                                        Ok((n, _)) if n > 0 => tracing::info!("mail FSEvents sync: {n} new"),
                                        Ok(_) => {}
                                        Err(e) => tracing::warn!("mail FSEvents sync error: {e}"),
                                    }
                                } else {
                                    tracing::debug!("mail FSEvents: WAL changed, skipped (battery)");
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

    let state = AppState {
        db,
        db_path: data_dir.join("bagent.db"),
        token,
        default_model: "qwen2.5:7b".to_string(),
        classifier_model: "qwen2.5:0.5b".to_string(),
        vision_model: "qwen2.5vl:7b".to_string(),
        attachments_dir,
        ollama,
        mail,
        notes,
        memory,
        prompt_builder,
        rules,
        pending_approvals,
    };

    let app = Router::new()
        .route("/health",                 get(health))
        .route("/models",                 get(models))
        .route("/chat",                   post(chat))
        .route("/embeddings",             post(embeddings))
        .route("/approvals/pending",      get(approvals_pending))
        .route("/approvals/:id/decide",   post(approval_decide))
        .route("/rules",                  get(rules_get).post(rules_save))
        // Phase 4B — Sessions
        .route("/sessions",               post(session_create).get(sessions_list))
        .route("/sessions/:id/turns",     get(session_turns))
        .route("/sessions/:id",           delete(session_delete))
        // Phase 4B — Memory
        .route("/memory",                 post(memory_insert).get(memory_list))
        .route("/memory/search",          get(memory_search))
        .route("/memory/:id",             delete(memory_delete))
        // Phase 5B — Attachments
        .route("/attachments",            post(upload_attachment))
        .route("/attachments/:id",        get(get_attachment))
        // Phase 4 — Mail
        .route("/mail/inbox",             get(mail_inbox))
        .route("/mail/message/:rowid",    get(mail_message))
        .route("/mail/sync",              post(mail_sync))
        // Phase 5C — Mail attachments
        .route("/mail/message/:rowid/attachments",      get(mail_message_attachments))
        .route("/mail/message/:rowid/attachments/:idx", get(mail_message_attachment_bytes))
        // Phase 5E — Open mail in Mail.app
        .route("/mail/open", post(mail_open))
        // Phase 4 — Notes
        .route("/notes/list",             get(notes_list))
        .route("/notes/search",           get(notes_search))
        .route("/notes/:pk",              get(notes_get))
        // Phase 4G — Disk usage
        .route("/usage",                  get(disk_usage))
        .route("/mail/cache/clear",       post(mail_cache_clear))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    std::fs::write(data_dir.join("daemon.port"), port.to_string())?;
    tracing::info!("bagentd listening on 127.0.0.1:{}", port);

    axum::serve(listener, app).await?;
    Ok(())
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

    let (memory_items_count, chat_turns_count, mail_cache_count, embeddings_count): (i64, i64, i64, i64) = {
        let db = state.db.lock().await;
        let mc: i64 = db.query_row("SELECT COUNT(*) FROM memory_items", [], |r| r.get(0)).unwrap_or(0);
        let ct: i64 = db.query_row("SELECT COUNT(*) FROM chat_turns", [], |r| r.get(0)).unwrap_or(0);
        let mail: i64 = db.query_row("SELECT COUNT(*) FROM mail_cache", [], |r| r.get(0)).unwrap_or(0);
        let emb: i64 = db.query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0)).unwrap_or(0);
        (mc, ct, mail, emb)
    };

    let total_bytes = db_bytes + attachments_bytes;

    (StatusCode::OK, Json(serde_json::json!({
        "db_bytes": db_bytes,
        "attachments_bytes": attachments_bytes,
        "memory_items_count": memory_items_count,
        "chat_turns_count": chat_turns_count,
        "mail_cache_count": mail_cache_count,
        "embeddings_count": embeddings_count,
        "total_bytes": total_bytes
    })))
}

async fn mail_cache_clear(State(state): State<AppState>) -> impl IntoResponse {
    let db = state.db.lock().await;
    let n = db.execute(
        "DELETE FROM mail_cache WHERE synced_at < strftime('%s', datetime('now', '-30 days'))",
        [],
    ).unwrap_or(0);
    (StatusCode::OK, Json(serde_json::json!({ "deleted": n })))
}

/// Returns true when the Mac is connected to AC power (not running on battery).
/// Uses `pmset -g batt` — fast, no extra deps. Falls back to true on error
/// so background tasks run as expected when power status is unknown.
fn is_on_ac_power() -> bool {
    let Ok(out) = std::process::Command::new("pmset")
        .args(["-g", "batt"])
        .output()
    else { return true; };
    let s = String::from_utf8_lossy(&out.stdout);
    // "Now drawing from 'AC Power'" or "'Battery Power'"
    s.contains("AC Power")
}

fn dir_size(path: &std::path::Path) -> u64 {
    if !path.exists() { return 0; }
    let Ok(entries) = std::fs::read_dir(path) else { return 0; };
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
    if !ok { return Err(StatusCode::UNAUTHORIZED); }
    Ok(next.run(req).await)
}

// ── Core handlers ─────────────────────────────────────────────────────────────

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        ollama: state.ollama.is_up().await,
        model: state.default_model,
        classifier_model: state.classifier_model,
        connectors: ConnectorStatus {
            mail:  state.mail.is_some(),
            notes: state.notes.is_some(),
        },
    })
}

async fn models(State(state): State<AppState>) -> impl IntoResponse {
    match state.ollama.models().await {
        Ok(names) => (StatusCode::OK, Json(serde_json::json!({ "models": names }))),
        Err(e)    => (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "error": e.to_string() }))),
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
        (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "approval not found or already decided" })))
    }
}

async fn rules_get(State(state): State<AppState>) -> impl IntoResponse {
    let yaml = state.rules.rules_yaml();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        yaml,
    )
}

async fn rules_save(
    State(state): State<AppState>,
    Json(req): Json<RulesSaveRequest>,
) -> impl IntoResponse {
    match state.rules.save_yaml(&req.yaml) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))),
    }
}

async fn embeddings(
    State(state): State<AppState>,
    Json(req): Json<EmbedRequest>,
) -> impl IntoResponse {
    let model = req.model.as_deref().unwrap_or(DEFAULT_EMBED_MODEL);
    match state.ollama.embed(model, &req.input).await {
        Ok(vec)  => (StatusCode::OK, Json(serde_json::json!({ "embedding": vec, "model": model }))),
        Err(e)   => (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "error": e.to_string() }))),
    }
}

async fn chat(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Sse<ReceiverStream<Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel(64);
    let model = req.model.clone().unwrap_or(state.default_model.clone());
    let classifier_model = state.classifier_model.clone();
    let db = state.db.clone();
    let ollama = state.ollama.clone();
    let user_message = req.message.clone();
    let mail  = state.mail.clone();
    let notes = state.notes.clone();
    let ctx_db = state.db.clone();
    let memory = state.memory.clone();
    let prompt_builder = state.prompt_builder.clone();
    let rules = state.rules.clone();
    let pending_approvals = state.pending_approvals.clone();
    let vision_model = state.vision_model.clone();
    let attachment_ids = req.attachment_ids.clone();

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
            if let Ok(Some(directive)) = directive_extractor.detect_and_extract(&user_message).await {
                if let Ok(Some(mem_id)) = memory.insert(
                    &directive.namespace,
                    &directive.kind,
                    &directive.language,
                    &directive.directive,
                    None, None, None,
                ).await {
                    let ev = Event::default().data(
                        serde_json::json!({"type":"memory_saved","id": mem_id}).to_string()
                    );
                    let _ = tx.send(Ok(ev)).await;
                }
            }
        }

        tracing::info!("chat timing: directive check {}ms", t0.elapsed().as_millis());
        // Load server-side history + session summary + last mail ref in parallel
        let (history, session_summary, last_mail_ref) = tokio::join!(
            async {
                if req.history.is_empty() {
                    load_session_history(&db, &session_id).await
                } else {
                    prepare_history(&ollama, &model, req.history).await
                }
            },
            load_session_summary(&db, &session_id),
            load_last_mail_ref(&db, &session_id),
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
        let needs_mail  = ["email", "mail", "správ", "inbox", "schránk", "doručen",
                           "posledn", "prečítaj", "read", "sender", "odosielate",
                           "nazvom", "názvom", "mailbox", "prilohu", "prílohu"]
            .iter().any(|kw| low.contains(kw));
        let needs_notes = ["poznámk", "note", "zápis", "zapisal", "napísal"]
            .iter().any(|kw| low.contains(kw));

        let mut allowed_tools: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for (needed, tool_name, description) in [
            (needs_mail,  "mail_inbox",  "Čítanie poštovej schránky (Apple Mail)"),
            (needs_notes, "notes_list",  "Čítanie poznámok (Apple Notes)"),
        ] {
            if !needed { continue; }
            match rules.check(tool_name, "{}") {
                ApprovalLevel::Auto => { allowed_tools.insert(tool_name.to_string()); }
                ApprovalLevel::Ask  => {
                    let approved = request_tool_approval(
                        &db, &pending_approvals, &tx, tool_name, description,
                    ).await;
                    if approved { allowed_tools.insert(tool_name.to_string()); }
                }
                ApprovalLevel::Forbidden => {
                    let _ = tx.send(Ok(Event::default().data(
                        serde_json::json!({"type":"tool_blocked","tool": tool_name}).to_string(),
                    ))).await;
                }
            }
        }

        tracing::info!("chat timing: rules checked {}ms", t0.elapsed().as_millis());
        // Fetch live tool context (mail/notes) only for approved tools
        let (tool_ctx, mail_pdf_paths, mail_ref_opt) = fetch_tool_context(
            &user_message, &history, last_mail_ref.as_ref(), &allowed_tools,
            ctx_db, mail, notes, ollama.clone(), classifier_model.clone(), memory.clone(),
        ).await;

        // Emit mail_found before tokens so the MailRef is in place when the client
        // starts watching for the auto-open trigger.
        if let Some(ref mail_ref) = mail_ref_opt {
            let _ = tx.send(Ok(Event::default().data(
                serde_json::json!({
                    "type": "mail_found",
                    "rowid": mail_ref.rowid,
                    "message_id": mail_ref.message_id,
                    "subject": mail_ref.subject,
                    "sender": mail_ref.sender,
                    "auto_open": mail_ref.auto_open,
                }).to_string()
            ))).await;
            // Persist for cross-turn reference ("tento mail", "má prílohy?")
            save_last_mail_ref(&db, &session_id, mail_ref).await;
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
                                ctx_parts.push(format!("### {} (obrázok — spracované modelom pre videnie)", filename));
                            } else {
                                let text = extracted_text.unwrap_or_else(|| "[obsah nedostupný]".to_string());
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

            let model_override = if has_image && req.model.is_none() {
                // Auto-route to vision model; log the swap
                if let Ok(db_guard) = db.try_lock() {
                    let _ = db_guard.execute(
                        "INSERT INTO audit_entries (action, payload, model) VALUES ('model_swap', ?1, ?2)",
                        rusqlite::params![
                            serde_json::json!({"from": model, "to": vision_model, "reason": "image_attachment"}).to_string(),
                            vision_model
                        ],
                    );
                }
                Some(vision_model)
            } else {
                None
            };

            AttachmentData { images_b64, ctx, model_override, turn_ids }
        };

        let effective_model = att_data.model_override.clone().unwrap_or(model.clone());

        tracing::info!("chat timing: tool_ctx fetched {}ms", t0.elapsed().as_millis());
        // Build layered prompt
        let messages = match prompt_builder
            .build(Some(&session_id), &user_message, lang, tool_ctx, att_data.ctx, history, session_summary)
            .await
        {
            Ok(mut msgs) => {
                if att_data.images_b64.is_empty() {
                    msgs.push(Message::user(&user_message));
                } else {
                    msgs.push(Message::user_with_images(&user_message, att_data.images_b64.clone()));
                }
                msgs
            }
            Err(_) => {
                if att_data.images_b64.is_empty() {
                    vec![Message::user(&user_message)]
                } else {
                    vec![Message::user_with_images(&user_message, att_data.images_b64.clone())]
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
            t0.elapsed().as_millis(), messages.len(), prompt_chars, prompt_chars / 4
        );
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
                    if tx.send(Ok(ev)).await.is_err() { return; }
                }
                Err(e) => {
                    let _ = tx.send(Ok(err_event(&e.to_string()))).await;
                    return;
                }
            }
        }

        // Emit mail attachment chips before done so the UI can show them
        if !mail_pdf_paths.is_empty() {
            let atts: Vec<serde_json::Value> = mail_pdf_paths.iter().map(|(fname, path)| {
                serde_json::json!({
                    "filename": fname,
                    "path": path.to_string_lossy(),
                    "size": std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
                })
            }).collect();
            let _ = tx.send(Ok(Event::default().data(
                serde_json::json!({"type":"mail_attachments","attachments": atts}).to_string()
            ))).await;
        }

        let _ = tx.send(Ok(Event::default().data(
            serde_json::json!({"type":"done","session_id": session_id}).to_string()
        ))).await;

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

        // Background: correction classifier + passive memory extraction + session summarizer + turn embedding
        let correction_classifier = CorrectionClassifier::new(ollama.clone(), classifier_model.clone());
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
                            let namespace = if result.scope == "sk_lang" { "sk_glossary" } else { "correction" };
                            let _ = mem.insert(namespace, "correction", "und", &text, None, None, None).await;
                        }
                    }
                }
            };

            let extract_fut = memory_extractor.run(&user_msg_bg, &reply_bg, memory_bg.clone(), &lang_bg);

            tokio::join!(embed_fut, correction_fut, extract_fut);

            // Session summarizer: every 10 turns, regenerate sessions.summary
            let turn_count: i64 = db_bg.try_lock().ok().and_then(|db| {
                db.query_row(
                    "SELECT COUNT(*) FROM chat_turns WHERE session_id = ?1",
                    rusqlite::params![session_bg],
                    |r| r.get(0),
                ).ok()
            }).unwrap_or(0);

            if turn_count > 0 && turn_count % 10 == 0 {
                // Fetch last 20 turns for summary
                let turns_text: Option<String> = db_bg.try_lock().ok().and_then(|db| {
                    let mut stmt = db.prepare(
                        "SELECT role, content FROM chat_turns WHERE session_id = ?1 \
                         ORDER BY created_at DESC LIMIT 20"
                    ).ok()?;
                    let rows: Vec<String> = stmt.query_map(rusqlite::params![session_bg], |r| {
                        let role: String = r.get(0)?;
                        let content: String = r.get(1)?;
                        Ok(format!("[{role}]: {content}"))
                    }).ok()?.flatten().collect();
                    Some(rows.into_iter().rev().collect::<Vec<_>>().join("\n"))
                });

                if let Some(text) = turns_text {
                    let prompt = format!(
                        "Summarize this conversation concisely in 2-3 sentences, preserving key facts and decisions:\n{text}"
                    );
                    if let Ok(summary) = ollama_bg.summarize(&model_bg, &[Message::user(prompt)]).await {
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
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "session_id": id }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
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
    (StatusCode::OK, Json(serde_json::json!({ "sessions": sessions })))
}

async fn session_turns(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let db = state.db.lock().await;
    let mut stmt = match db.prepare(
        "SELECT id, role, content, language, model, created_at FROM chat_turns \
         WHERE session_id = ?1 ORDER BY created_at"
    ) {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
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
        Ok(_) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "session not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }
}

// ── Memory handlers ───────────────────────────────────────────────────────────

async fn memory_insert(
    State(state): State<AppState>,
    Json(req): Json<MemoryInsertRequest>,
) -> impl IntoResponse {
    match state.memory.insert(
        &req.namespace,
        &req.kind,
        &req.language,
        &req.text,
        req.source_ref.as_deref(),
        req.metadata_json.as_deref(),
        req.expires_at.as_deref(),
    ).await {
        Ok(Some(id)) => {
            let db = state.db.lock().await;
            let _ = db.execute(
                "INSERT INTO audit_entries (action, payload, model) VALUES ('memory_save', ?1, '')",
                rusqlite::params![serde_json::json!({"id": id, "kind": req.kind, "namespace": req.namespace}).to_string()],
            );
            (StatusCode::OK, Json(serde_json::json!({ "id": id, "saved": true })))
        }
        Ok(None)     => (StatusCode::OK, Json(serde_json::json!({ "saved": false, "reason": "duplicate" }))),
        Err(e)       => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }
}

async fn memory_list(
    State(state): State<AppState>,
    Query(q): Query<MemorySearchQuery>,
) -> impl IntoResponse {
    let db = state.db.lock().await;
    let sql = if q.namespace.is_empty() {
        "SELECT id, namespace, kind, language, text, source_ref, created_at, use_count \
         FROM memory_items ORDER BY updated_at DESC LIMIT ?1".to_string()
    } else {
        "SELECT id, namespace, kind, language, text, source_ref, created_at, use_count \
         FROM memory_items WHERE namespace = ?2 ORDER BY updated_at DESC LIMIT ?1".to_string()
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
        }))
    };

    let items: Vec<serde_json::Value> = if q.namespace.is_empty() {
        db.prepare(&sql).ok()
            .and_then(|mut s| s.query_map(rusqlite::params![q.limit as i64], query_fn).ok()
                .map(|rows| rows.flatten().collect()))
            .unwrap_or_default()
    } else {
        db.prepare(&sql).ok()
            .and_then(|mut s| s.query_map(rusqlite::params![q.limit as i64, q.namespace], query_fn).ok()
                .map(|rows| rows.flatten().collect()))
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

    match state.memory.retrieve(&q.q, &namespaces, q.limit).await {
        Ok(hits) => {
            let items: Vec<serde_json::Value> = hits
                .into_iter()
                .map(|h| serde_json::json!({
                    "id": h.item.id,
                    "namespace": h.item.namespace,
                    "kind": h.item.kind,
                    "text": h.item.text,
                    "score": h.score,
                }))
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "hits": items })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }
}

async fn memory_delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.memory.delete(&id) {
        Ok(true)  => {
            let db = state.db.lock().await;
            let _ = db.execute(
                "INSERT INTO audit_entries (action, payload, model) VALUES ('memory_forget', ?1, '')",
                rusqlite::params![serde_json::json!({"id": id}).to_string()],
            );
            (StatusCode::OK, Json(serde_json::json!({ "deleted": true })))
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "not found" }))),
        Err(e)    => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
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
            Json(serde_json::json!({ "error": "Mail connector not accessible. Grant Full Disk Access in System Settings → Privacy & Security." })),
        );
    };

    match tokio::task::spawn_blocking(move || mail.list_inbox(q.limit, q.unread)).await {
        Ok(Ok(msgs)) => (StatusCode::OK, Json(serde_json::json!({ "messages": msgs }))),
        Ok(Err(e))   => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
        Err(e)       => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }
}

async fn mail_message(
    State(state): State<AppState>,
    Path(rowid): Path<i64>,
) -> impl IntoResponse {
    let Some(mail) = state.mail else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Mail connector not accessible." })),
        );
    };

    let mut msg = match tokio::task::spawn_blocking(move || mail.get_message(rowid)).await {
        Ok(Ok(Some(m))) => m,
        Ok(Ok(None))    => return (StatusCode::NOT_FOUND,            Json(serde_json::json!({ "error": "message not found" }))),
        Ok(Err(e))      => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
        Err(e)          => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    };

    // emlx not locally cached → try AppleScript fallback (needs Automation → Mail)
    if msg.body.is_none() {
        if let Some(body) = apple_mail_connector::body_via_applescript(&msg.subject).await {
            msg.language = apple_mail_connector::detect_language(&body);
            msg.body = Some(body);
            msg.body_available = true;
        }
    }

    (StatusCode::OK, Json(serde_json::json!({ "message": msg, "pii": true })))
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
            .await.ok()
            .and_then(|r| r.ok())
            .flatten()
            .and_then(|m| m.message_id)
    } else {
        None
    };

    match apple_mail_connector::open_message(
        message_id.as_deref(),
        &req.subject,
        &req.sender,
    ).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "opened": true }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
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
        Ok(Ok(Some(msg))) => (StatusCode::OK, Json(serde_json::json!({ "attachments": msg.attachments }))),
        Ok(Ok(None))      => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "message not found" }))),
        Ok(Err(e))        => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
        Err(e)            => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
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
    match tokio::task::spawn_blocking(move || mail.get_message_attachment_base64(rowid, idx)).await {
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
        Ok(Err(e)) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": e.to_string() }))),
        Err(e)     => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
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
            if name != "file" { continue; }
        }
        if let Some(fn_) = field.file_name() { filename = fn_.to_string(); }
        if let Some(ct) = field.content_type() { mime = ct.to_string(); }

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
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "no file field in multipart" })));
    }

    // Compute SHA-256 for content-addressed storage
    let sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&file_bytes);
        format!("{:x}", hasher.finalize())
    };

    // Derive file extension from filename / MIME
    let ext = filename.rsplit('.').next()
        .filter(|e| e.len() <= 6 && e.chars().all(|c| c.is_alphanumeric()))
        .unwrap_or("bin");
    let stored_name = format!("{sha256}.{ext}");
    let bytes_path = state.attachments_dir.join(&stored_name);

    // Write file (idempotent — same sha → same path, no overwrite needed)
    if !bytes_path.exists() {
        if let Err(e) = std::fs::write(&bytes_path, &file_bytes) {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })));
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
    let existing_id: Option<String> = db.query_row(
        "SELECT id FROM attachments WHERE sha256 = ?1",
        rusqlite::params![sha256],
        |r| r.get(0),
    ).ok();

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

    (StatusCode::OK, Json(serde_json::json!({
        "attachment_id": att_id,
        "filename": filename,
        "mime": mime,
        "kind": kind,
        "size": size_bytes,
        "sha256": sha256,
    })))
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
                    return (StatusCode::OK, Json(serde_json::json!({
                        "id": id,
                        "filename": filename,
                        "mime": mime,
                        "size": size,
                        "data_base64": B64.encode(&bytes),
                    })));
                }
            }
            (StatusCode::OK, Json(serde_json::json!({
                "id": id,
                "filename": filename,
                "mime": mime,
                "size": size,
                "extracted_text": extracted_text,
            })))
        }
        Err(_) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "attachment not found" }))),
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
            Json(serde_json::json!({ "error": "Notes connector not accessible. Grant Full Disk Access in System Settings → Privacy & Security." })),
        );
    };

    match tokio::task::spawn_blocking(move || notes.list_notes(q.limit)).await {
        Ok(Ok(items)) => (StatusCode::OK, Json(serde_json::json!({ "notes": items }))),
        Ok(Err(e))    => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
        Err(e)        => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
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
        Ok(Err(e))    => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
        Err(e)        => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }
}

async fn notes_get(
    State(state): State<AppState>,
    Path(pk): Path<i64>,
) -> impl IntoResponse {
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
    }).await {
        Ok(Ok(Some(n))) => n,
        Ok(Ok(None))    => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "note not found" }))),
        Ok(Err(e))      => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
        Err(e)          => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    };

    if meta.is_locked {
        return (StatusCode::OK, Json(serde_json::json!({ "note": meta, "pii": true })));
    }

    // Fetch body via JXA
    let coredata_id = meta.coredata_id.clone();
    let body = notes.get_note_body(&coredata_id).await.ok().flatten();
    let lang = body.as_deref().and_then(apple_notes_connector::detect_language);

    let mut note = meta;
    note.body = body;
    note.language = lang;

    (StatusCode::OK, Json(serde_json::json!({ "note": note, "pii": true })))
}

// ── Approval helper ──────────────────────────────────────────────────────────

/// Insert a pending_approvals record, emit an SSE event to the client, and
/// block until the user decides (Allow/Deny) or the 60 s countdown elapses.
async fn request_tool_approval(
    db: &Arc<Mutex<Connection>>,
    pending: &Arc<std::sync::Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    tx: &mpsc::Sender<Result<Event, Infallible>>,
    tool_name: &str,
    description: &str,
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
            [], |r| r.get(0),
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
        db_lock.execute(
            "INSERT INTO connectors (kind, config_json, enabled, last_sync_at)
             VALUES ('apple_mail','{}',1,?1)
             ON CONFLICT(kind) DO UPDATE SET last_sync_at = ?1",
            rusqlite::params![now],
        ).ok();
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
        return (StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "Mail connector not accessible." })));
    };

    match mail_sync_inner(state.db.clone(), mail, state.memory.clone()).await {
        Ok((count, now)) => {
            let total: i64 = {
                let db = state.db.lock().await;
                db.query_row("SELECT COUNT(*) FROM mail_cache", [], |r| r.get(0)).unwrap_or(0)
            };
            (StatusCode::OK, Json(serde_json::json!({
                "synced": count,
                "total_cached": total,
                "last_sync_at": now
            })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))),
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
    let end   = Utc.from_utc_datetime(&d.and_hms_opt(23, 59, 59)?).timestamp() + 1;
    Some((start, end))
}

/// Helper: fetch recent messages, merging mail_cache with live Envelope Index.
/// Always queries both sources so very-recent (not-yet-synced) mails appear.
async fn fetch_recent_mails(
    db: &Arc<Mutex<Connection>>,
    mail: &Option<MailConnector>,
    limit: usize,
) -> Vec<apple_mail_connector::MailMessage> {
    let cached: Vec<apple_mail_connector::MailMessage> = {
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
            .await.ok().and_then(|r| r.ok()).unwrap_or_default()
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
            .await.ok().and_then(|r| r.ok()).flatten()
    };
    let Some(mut full_msg) = full else { return vec![] };
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
        komu_fallback = full_msg.mailbox_url
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
         polia vlastnými odhadmi, nemiešaj s inými rozhovormi ani kontextami.".to_string()
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
            if att_meta.mimetype.starts_with("image/") { continue; }
            let mc2 = mc_clone.clone();
            let part_idx = att_meta.part_index;
            let filename = att_meta.filename.clone();
            let mime = att_meta.mimetype.clone();
            let bytes_result = tokio::task::spawn_blocking(move || {
                mc2.get_message_attachment(rowid, part_idx)
            }).await.ok().and_then(|r| r.ok());

            if let Some((_, bytes)) = bytes_result {
                let tmp = std::env::temp_dir().join(format!("bagent_mail_{}", &filename));
                if std::fs::write(&tmp, &bytes).is_ok() {
                    let extracted = extract_attachment(&tmp, &mime)
                        .ok().and_then(|r| r.extracted_text);
                    if let Some(text) = extracted {
                        lines.push(format!("\n\n**Obsah prílohy ({filename}):**\n\n{text}"));
                    } else {
                        lines.push(format!("\n\n**Príloha:** {filename} (obsah nedostupný na analýzu)"));
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
            } else { body.clone() };
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
            let label = if m.role == "user" { "[User]" } else { "[Assistant]" };
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
    allowed_tools: &std::collections::HashSet<String>,
    db: Arc<Mutex<Connection>>,
    mail: Option<MailConnector>,
    notes: Option<NotesConnector>,
    ollama: OllamaClient,
    model: String,
    memory: Arc<MemoryStore>,
) -> (Option<String>, Vec<(String, std::path::PathBuf)>, Option<MailRef>) {
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
            .iter().any(|kw| low.contains(kw));

    let history_snippet = {
        let base = format_history_snippet(history, 4);
        match last_mail_ref {
            Some(r) => format!(
                "[LastFoundMail]: rowid={} sender=\"{}\" subject=\"{}\"\n{}",
                r.rowid, r.sender, r.subject, base
            ),
            None => base,
        }
    };

    let mut parts: Vec<String> = Vec::new();
    let mut pdf_paths: Vec<(String, std::path::PathBuf)> = Vec::new();
    let mut found_mail_ref: Option<MailRef> = None;

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
                    let lines: Vec<String> = msgs.iter().map(|m| {
                        let from = m.sender_display.as_deref().unwrap_or(&m.sender);
                        let status = if m.is_read { "✓" } else { "●" };
                        format!("  {status} [{}] Od: {} | Predmet: {}",
                            relative_date(m.received_at), from, m.subject)
                    }).collect();
                    parts.push(format!("Posledné emaily (Apple Mail):\n{}", lines.join("\n")));
                } else {
                    parts.push("Posledné emaily (Apple Mail): žiadne správy nenájdené.".to_string());
                }
            }

            "search" | "read_attachment" | "open" => {
                let is_attachment = intent.action == "read_attachment" || intent.wants_attachment;
                let wants_open = intent.action == "open"
                    || ["otvor", "otvoriť", "open it", "show me", "ukáž mi"]
                        .iter().any(|kw| low.contains(kw));

                // Short-circuit: "má tento mail prílohy?" with no new search filters
                // → use the last found mail's rowid directly, skip re-search.
                let no_search_filters = intent.sender.is_none()
                    && intent.subject.is_none()
                    && intent.date.is_none()
                    && intent.keywords.is_empty();
                if is_attachment && no_search_filters {
                    if let Some(ref lmr) = last_mail_ref {
                        tracing::info!("attachment short-circuit via last_mail_ref rowid={}", lmr.rowid);
                        if let Some(ref mc) = mail {
                            let mc2 = mc.clone();
                            let rowid = lmr.rowid;
                            // get_message populates attachments from emlx parsing
                            let full_msg = tokio::task::spawn_blocking(move || mc2.get_message(rowid))
                                .await.ok().and_then(|r| r.ok()).flatten();

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
                                let att_lines: Vec<String> = attach_list.iter().map(|a| {
                                    format!("  • {} ({}, {} B)", a.filename, a.mimetype, a.size)
                                }).collect();
                                parts.push(format!(
                                    "Prílohy emailu \"{}\" (od: {}):\n{}",
                                    lmr.subject, lmr.sender, att_lines.join("\n")
                                ));
                                // Stage PDF attachment paths for chips
                                for (idx, a) in attach_list.iter().enumerate() {
                                    if a.mimetype.contains("pdf") {
                                        if let Some(ref mc3) = mail {
                                            let mc3 = mc3.clone();
                                            let rid = lmr.rowid;
                                            if let Ok(Some((_, bytes))) = tokio::task::spawn_blocking(move || {
                                                mc3.get_message_attachment(rid, idx).ok()
                                            }).await {
                                                let tmp = std::env::temp_dir()
                                                    .join(format!("bagent_att_{}_{}.pdf", rid, idx));
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

                let (date_from, date_to) = intent.date.as_deref()
                    .and_then(parse_date_to_range)
                    .map(|(s, e)| (Some(s), Some(e)))
                    .unwrap_or((None, None));

                // When user says "recent"/"posledné"/"nové" but no explicit date, add a
                // 30-day window so we don't scan the entire inbox history.
                let date_from = date_from.or_else(|| {
                    let has_recent_word = ["recent", "posledn", "nové", "najnov", "latest", "new"]
                        .iter().any(|kw| low.contains(kw));
                    if has_recent_word && intent.date.is_none() {
                        Some(chrono::Utc::now().timestamp() - 30 * 24 * 3600)
                    } else {
                        None
                    }
                });

                // Stop-words that should never be used as SQL search filters.
                let mail_stopwords = ["recent", "mail", "email", "new", "latest", "inbox",
                                      "nové", "posledn", "správ", "schránk"];
                let meaningful_keywords: Vec<String> = intent.keywords.iter()
                    .filter(|kw| !mail_stopwords.iter().any(|sw| kw.to_lowercase().contains(sw)))
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
                let filter_keywords: Vec<String> = if effective_sender.is_some() || intent.subject.is_some() {
                    vec![]
                } else {
                    meaningful_keywords
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
                let filter_sender_fb = filter.sender.clone();
                let filter_subject_fb = filter.subject.clone();
                let filter_keywords_fb = filter.keywords.clone();
                let msgs: Vec<apple_mail_connector::MailMessage> = if let Some(ref mc) = mail {
                    let mc = mc.clone();
                    tokio::task::spawn_blocking(move || mc.search_messages(&filter))
                        .await.ok().and_then(|r| r.ok()).unwrap_or_default()
                } else {
                    vec![]
                };

                // Fallback: if no results and we had a date filter, retry without it.
                let msgs = if msgs.is_empty() && had_date_filter {
                    let wider = MailSearchFilter {
                        sender: filter_sender_fb,
                        subject: filter_subject_fb,
                        date_from: None,
                        date_to: None,
                        limit: 10,
                        keywords: filter_keywords_fb,
                    };
                    if let Some(ref mc) = mail {
                        let mc = mc.clone();
                        tokio::task::spawn_blocking(move || mc.search_messages(&wider))
                            .await.ok().and_then(|r| r.ok()).unwrap_or_default()
                    } else {
                        vec![]
                    }
                } else {
                    msgs
                };

                if msgs.is_empty() {
                    let mut why = Vec::new();
                    if let Some(ref s) = intent.sender  { why.push(format!("odosielateľ: {s}")); }
                    if let Some(ref s) = intent.subject { why.push(format!("predmet: {s}")); }
                    if let Some(ref d) = intent.date    { why.push(format!("dátum: {d}")); }
                    parts.push(format!(
                        "Vyhľadávanie v Apple Mail ({}) — žiadny email nenájdený. \
                         Email neexistuje v lokálnej schránke alebo nebol stiahnutý cez IMAP.",
                        if why.is_empty() { "bez filtrov".to_string() } else { why.join(", ") }
                    ));
                } else {
                    let mut lines: Vec<String> = msgs.iter().map(|m| {
                        let from = m.sender_display.as_deref().unwrap_or(&m.sender);
                        let status = if m.is_read { "✓" } else { "●" };
                        format!("  {status} [{}] Od: {} <{}> | Predmet: {}",
                            relative_date(m.received_at), from, m.sender, m.subject)
                    }).collect();

                    // Find best match: prefer message whose body contains a keyword.
                    let mut best_rowid = msgs[0].rowid;
                    if !intent.keywords.is_empty() {
                        'outer: for msg_item in &msgs {
                            if let Some(ref mc) = mail {
                                let mc2 = mc.clone();
                                let rid = msg_item.rowid;
                                if let Some(full) = tokio::task::spawn_blocking(move || mc2.get_message(rid))
                                    .await.ok().and_then(|r| r.ok()).flatten()
                                {
                                    if let Some(ref body) = full.body {
                                        let body_low = body.to_lowercase();
                                        if intent.keywords.iter().any(|kw| body_low.contains(kw.as_str())) {
                                            best_rowid = msg_item.rowid;
                                            break 'outer;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let found = enrich_message(best_rowid, &mail, is_attachment, &mut lines).await;
                    pdf_paths.extend(found);

                    let best_msg = msgs.iter().find(|m| m.rowid == best_rowid).unwrap_or(&msgs[0]);
                    found_mail_ref = Some(MailRef {
                        rowid: best_rowid,
                        message_id: None,
                        subject: best_msg.subject.clone(),
                        sender: best_msg.sender.clone(),
                        auto_open: wants_open,
                    });

                    parts.push(format!("Nájdené emaily (Apple Mail):\n{}", lines.join("\n")));
                }
                } // end if !already_handled
            }

            _ => {
                let msgs = fetch_recent_mails(&db, &mail, 10).await;
                if !msgs.is_empty() {
                    let lines: Vec<String> = msgs.iter().map(|m| {
                        let from = m.sender_display.as_deref().unwrap_or(&m.sender);
                        let status = if m.is_read { "✓" } else { "●" };
                        format!("  {status} [{}] Od: {} | Predmet: {}",
                            relative_date(m.received_at), from, m.subject)
                    }).collect();
                    parts.push(format!("Posledné emaily (Apple Mail):\n{}", lines.join("\n")));
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
                    let lines: Vec<String> = items.iter().map(|note| {
                        let folder = note.folder.as_deref().unwrap_or("Notes");
                        let snip = note.snippet.as_deref().unwrap_or("");
                        format!("  [{}] {} — {}", folder, note.title, snip)
                    }).collect();
                    parts.push(format!("Posledné poznámky (Apple Notes):\n{}", lines.join("\n")));
                }
            }
        }
    }

    // ── AeroSpace window management ───────────────────────────────────────────
    // Cheap keyword gate — only invoke the classifier when the turn looks
    // like a window/workspace management request.
    let looks_like_window = ["plochu", "ploch", "workspace", "prepni", "presuň",
                             "presun", "zameraj", "otvor na ploch"]
        .iter().any(|kw| low.contains(kw));

    if looks_like_window {
        if let Ok(intent) = WindowIntentClassifier::new(ollama, model)
            .classify(message, &history_snippet).await
        {
            tracing::debug!("window_intent: {:?}", intent);
            if intent.action != "none" {
                if let Ok(note) = run_aerospace_intent(&intent).await {
                    if !note.is_empty() {
                        parts.push(note);
                    }
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

    (ctx, pdf_paths, found_mail_ref)
}

// ── AeroSpace executor ────────────────────────────────────────────────────────

/// Resolve the `aerospace` binary path: try $PATH first, then the bundled
/// in-app binary. Returns `None` if AeroSpace is not installed.
async fn find_aerospace_binary() -> Option<std::path::PathBuf> {
    // Try $PATH via `which`
    if let Ok(out) = tokio::process::Command::new("which")
        .arg("aerospace")
        .output().await
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(std::path::PathBuf::from(s));
            }
        }
    }
    // Bundled fallback
    let bundled = std::path::PathBuf::from(
        "/Applications/AeroSpace.app/Contents/Resources/aerospace"
    );
    if bundled.exists() { Some(bundled) } else { None }
}

/// Run an `aerospace` subcommand. Returns `Ok(stdout)` on success,
/// `Err` on binary-not-found or non-zero exit (caller logs and silently degrades).
async fn run_aerospace(args: &[&str]) -> anyhow::Result<String> {
    let bin = find_aerospace_binary().await
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
                    Ok(_)  => return Ok(format!("Prepnuté na plochu {ws}.")),
                    Err(e) => { tracing::warn!("aerospace focus_workspace: {e}"); return Ok(String::new()); }
                }
            }
        }

        "open_app" => {
            let app = intent.app.as_deref().unwrap_or("Mail");
            // 1. Launch (or focus) the application
            let _ = tokio::process::Command::new("open")
                .args(["-a", app])
                .output().await;

            if let Some(ref ws) = intent.workspace {
                // 2. Poll until the window appears, then move it
                let bundle_id = app_to_bundle_id(app);
                let ws_str = ws.clone();
                let app_str = app.to_string();
                tokio::spawn(async move {
                    for _ in 0..30 {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        let Ok(list) = run_aerospace(&[
                            "list-windows", "--all",
                            "--app-bundle-id", &bundle_id,
                            "--format", "%{window-id}",
                        ]).await else { continue };
                        let wid = list.lines().next().unwrap_or("").trim().to_string();
                        if wid.is_empty() { continue; }
                        // Try with --window-id first (AeroSpace 0.15+), fall back to focus+move
                        let moved = run_aerospace(&[
                            "move-node-to-workspace", "--window-id", &wid, &ws_str,
                        ]).await;
                        if moved.is_err() {
                            let _ = run_aerospace(&["focus", "--window-id", &wid]).await;
                            let _ = run_aerospace(&["move-node-to-workspace", &ws_str]).await;
                        }
                        tracing::info!("aerospace: moved {app_str} (wid={wid}) to workspace {ws_str}");
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
            let ws  = intent.workspace.as_deref().unwrap_or("1");
            let bundle_id = app_to_bundle_id(app);
            let Ok(list) = run_aerospace(&[
                "list-windows", "--all",
                "--app-bundle-id", &bundle_id,
                "--format", "%{window-id}",
            ]).await else {
                tracing::warn!("aerospace move_app: could not list windows");
                return Ok(String::new());
            };
            let wid = list.lines().next().unwrap_or("").trim().to_string();
            if !wid.is_empty() {
                let moved = run_aerospace(&[
                    "move-node-to-workspace", "--window-id", &wid, ws,
                ]).await;
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
                "list-windows", "--all",
                "--app-bundle-id", &bundle_id,
                "--format", "%{window-id}",
            ]).await else {
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
        "mail"     => "com.apple.mail".to_string(),
        "safari"   => "com.apple.Safari".to_string(),
        "notes"    => "com.apple.Notes".to_string(),
        "finder"   => "com.apple.finder".to_string(),
        "terminal" => "com.apple.Terminal".to_string(),
        "xcode"    => "com.apple.dt.Xcode".to_string(),
        "vscode" | "visual studio code" => "com.microsoft.VSCode".to_string(),
        "slack"    => "com.tinyspeck.slackmacgap".to_string(),
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
        0        => "práve teraz".to_string(),
        1        => "pred 1 hodinou".to_string(),
        2..=23   => format!("pred {} hodinami", diff / 3600),
        24..=47  => "včera".to_string(),
        hours    => format!("pred {} dňami", hours / 24),
    }
}

// ── Context management ────────────────────────────────────────────────────────

async fn load_session_summary(db: &Arc<Mutex<Connection>>, session_id: &str) -> Option<String> {
    db.lock().await
        .query_row(
            "SELECT summary FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
}

async fn save_last_mail_ref(db: &Arc<Mutex<Connection>>, session_id: &str, mail_ref: &MailRef) {
    if let Ok(json) = serde_json::to_string(&serde_json::json!({ "last_mail_ref": mail_ref })) {
        let _ = db.lock().await.execute(
            "UPDATE sessions SET metadata_json = ?1 WHERE id = ?2",
            rusqlite::params![json, session_id],
        );
    }
}

async fn load_last_mail_ref(db: &Arc<Mutex<Connection>>, session_id: &str) -> Option<MailRef> {
    let json: Option<String> = db.lock().await
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

async fn load_session_history(db: &Arc<Mutex<Connection>>, session_id: &str) -> Vec<Message> {
    let db = db.lock().await;
    let mut stmt = match db.prepare(
        "SELECT role, content FROM chat_turns \
         WHERE session_id = ?1 ORDER BY created_at DESC LIMIT 10"
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let mut turns: Vec<Message> = stmt
        .query_map(rusqlite::params![session_id], |row| {
            let role: String = row.get(0)?;
            let content: String = row.get(1)?;
            Ok(match role.as_str() {
                "user"      => Message::user(content),
                "assistant" => Message::assistant(content),
                _           => Message::system(content),
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
        let old    = &history[..split];
        let recent = history[split..].to_vec();

        if let Ok(summary) = ollama.summarize(model, old).await {
            let mut result = vec![Message::system(
                format!("Zhrnutie predchádzajúcej konverzácie: {summary}")
            )];
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
