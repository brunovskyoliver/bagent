use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{delete, get, post},
    Json,
};
use bagent_agent::{
    CorrectionClassifier, DirectiveExtractor, PromptBuilder,
    has_explicit_trigger,
};
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
const MAX_HISTORY: usize = 40;

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    token: String,
    default_model: String,
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

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    ollama: bool,
    model: String,
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
        token,
        default_model: "qwen2.5:7b".to_string(),
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
        // Phase 4 — Mail
        .route("/mail/inbox",             get(mail_inbox))
        .route("/mail/message/:rowid",    get(mail_message))
        .route("/mail/sync",              post(mail_sync))
        // Phase 4 — Notes
        .route("/notes/list",             get(notes_list))
        .route("/notes/search",           get(notes_search))
        .route("/notes/:pk",              get(notes_get))
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

    tokio::spawn(async move {
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
        let directive_extractor = DirectiveExtractor::new(ollama.clone(), model.clone());
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

        // Load server-side history for this session (prefer over client-sent)
        let history = if req.history.is_empty() {
            load_session_history(&db, &session_id).await
        } else {
            prepare_history(&ollama, &model, req.history).await
        };

        // Detect language (simple heuristic: SK diacritics present?)
        let lang = if user_message.chars().any(|c| "áčďéíľĺňóôŕšťúýž".contains(c)) {
            "sk"
        } else {
            "en"
        };

        // Determine which tools are needed and gate via rules engine
        let low = user_message.to_lowercase();
        let needs_mail  = ["email", "mail", "správ", "inbox", "schránk", "doručen",
                           "posledn", "prečítaj", "read", "sender", "odosielate"]
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

        // Fetch live tool context (mail/notes) only for approved tools
        let tool_ctx = fetch_tool_context(&user_message, &allowed_tools, ctx_db, mail, notes).await;

        // Build layered prompt
        let messages = match prompt_builder
            .build(Some(&session_id), &user_message, lang, tool_ctx, history, None)
            .await
        {
            Ok(mut msgs) => {
                msgs.push(Message::user(&user_message));
                msgs
            }
            Err(_) => vec![Message::user(&user_message)],
        };

        // Persist user turn
        {
            let turn_id = Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            if let Ok(db) = db.try_lock() {
                let _ = db.execute(
                    "INSERT INTO chat_turns (id, session_id, role, content, language, model, created_at) \
                     VALUES (?1,?2,'user',?3,?4,?5,?6)",
                    rusqlite::params![turn_id, session_id, user_message, lang, model, now],
                );
            }
        }

        // Stream response
        let token_stream = ollama.chat_stream(model.clone(), messages);
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
                rusqlite::params![turn_id, session_id, full_response, lang, model, now],
            );
            let _ = db.execute(
                "INSERT INTO audit_entries (action, payload, model) VALUES (?1, ?2, ?3)",
                rusqlite::params!["chat", &user_message, &model],
            );
        }

        // Background: classify whether user corrected the assistant
        let correction_classifier = CorrectionClassifier::new(ollama.clone(), model.clone());
        let memory_bg = memory.clone();
        let user_msg_bg = user_message.clone();
        tokio::spawn(async move {
            if let Ok(result) = correction_classifier.classify(&response_for_audit, &user_msg_bg).await {
                if result.is_correction && result.confidence > 0.7 {
                    let text = format!(
                        "Oprava: {} → {}",
                        result.what_was_wrong.as_deref().unwrap_or("?"),
                        result.correct_behavior.as_deref().unwrap_or("?")
                    );
                    let namespace = if result.scope == "sk_lang" { "sk_glossary" } else { "correction" };
                    let _ = memory_bg.insert(namespace, "correction", "und", &text, None, None, None).await;
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

async fn mail_sync(State(state): State<AppState>) -> impl IntoResponse {
    let Some(mail) = state.mail.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "Mail connector not accessible." })));
    };

    // Last sync timestamp from connectors table
    let last_sync: i64 = {
        let db = state.db.lock().await;
        db.query_row(
            "SELECT COALESCE(last_sync_at, 0) FROM connectors WHERE kind = 'apple_mail'",
            [], |r| r.get(0),
        ).unwrap_or(0)
    };

    let new_msgs = match tokio::task::spawn_blocking(move || mail.list_since(last_sync, 500)).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
        Err(e)     => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    };

    let count = new_msgs.len();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    {
        let db = state.db.lock().await;
        for msg in &new_msgs {
            db.execute(
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
        db.execute(
            "INSERT INTO connectors (kind, config_json, enabled, last_sync_at)
             VALUES ('apple_mail','{}',1,?1)
             ON CONFLICT(kind) DO UPDATE SET last_sync_at = ?1",
            rusqlite::params![now],
        ).ok();
    }

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

// ── Tool context injection ────────────────────────────────────────────────────

/// Detects intent in the user message and pre-fetches Mail / Notes data so the
/// LLM can answer without needing explicit tool calls.
/// Reads from mail_cache when available (no Envelope Index access needed);
/// falls back to a live query when cache is empty.
async fn fetch_tool_context(
    message: &str,
    allowed_tools: &std::collections::HashSet<String>,
    db: Arc<Mutex<Connection>>,
    mail: Option<MailConnector>,
    notes: Option<NotesConnector>,
) -> Option<String> {
    let low = message.to_lowercase();

    // SK + EN keyword sets, gated by rule-approved tool list
    let wants_mail = allowed_tools.contains("mail_inbox")
        && ["email", "mail", "správ", "inbox", "schránk", "doručen",
            "posledn", "prečítaj", "read", "sender", "odosielate"]
            .iter().any(|kw| low.contains(kw));

    let wants_body = wants_mail
        && ["prečítaj", "obsah", "read", "body", "text", "čo hovorí",
            "what does", "what did", "detail", "celý"]
            .iter().any(|kw| low.contains(kw));

    let wants_notes = allowed_tools.contains("notes_list")
        && ["poznámk", "note", "zápis", "zapisal", "napísal"]
            .iter().any(|kw| low.contains(kw));

    let mut parts: Vec<String> = Vec::new();

    // ── Mail ─────────────────────────────────────────────────────────────────
    if wants_mail {
        // Prefer the local cache; fall back to live Envelope Index query.
        let msgs: Vec<apple_mail_connector::MailMessage> = {
            let db_lock = db.lock().await;
            let cached: Vec<_> = db_lock
                .prepare("SELECT rowid, subject, sender, sender_display, received_at, is_read, mailbox_url FROM mail_cache ORDER BY received_at DESC LIMIT 5")
                .and_then(|mut s| {
                    s.query_map([], |r| {
                        let display: Option<String> = r.get(3)?;
                        Ok(apple_mail_connector::MailMessage {
                            rowid: r.get(0)?,
                            subject: r.get(1)?,
                            sender: r.get(2)?,
                            sender_display: display,
                            received_at: r.get(4)?,
                            is_read: r.get::<_, i64>(5)? != 0,
                            mailbox_url: r.get(6)?,
                            body: None,
                            body_available: true,
                            language: None,
                        })
                    }).map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();
            cached
        };

        // Cache miss → live query
        let msgs = if msgs.is_empty() {
            if let Some(ref mc) = mail {
                let mc = mc.clone();
                tokio::task::spawn_blocking(move || mc.list_inbox(5, false))
                    .await.ok().and_then(|r| r.ok()).unwrap_or_default()
            } else { vec![] }
        } else { msgs };

        if !msgs.is_empty() {
            let mut lines: Vec<String> = msgs.iter().map(|msg| {
                let from = msg.sender_display.as_deref().unwrap_or(&msg.sender);
                let status = if msg.is_read { "✓" } else { "●" };
                format!("  {status} [{}] Od: {} | Predmet: {}",
                    relative_date(msg.received_at), from, msg.subject)
            }).collect();

            // Fetch body of the most recent when user wants to read content
            if wants_body {
                if let Some(first) = msgs.first() {
                    let rowid = first.rowid;
                    let subject = first.subject.clone();
                    let body_opt = if let Some(ref mc) = mail {
                        let mc = mc.clone();
                        let full = tokio::task::spawn_blocking(move || mc.get_message(rowid))
                            .await.ok().and_then(|r| r.ok()).flatten();
                        if let Some(mut m) = full {
                            // AppleScript fallback when emlx not cached
                            if m.body.is_none() {
                                m.body = apple_mail_connector::body_via_applescript(&subject).await;
                            }
                            m.body
                        } else { None }
                    } else { None };

                    if let Some(body) = body_opt {
                        let truncated = if body.len() > 1500 {
                            format!("{}…[skrátené]", &body[..1500])
                        } else { body };
                        lines.push(format!("\n  Obsah posledného emailu:\n{truncated}"));
                    }
                }
            }

            parts.push(format!("Posledné emaily (Apple Mail):\n{}", lines.join("\n")));
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

    if parts.is_empty() {
        None
    } else {
        Some(format!(
            "## Živé dáta z tvojich aplikácií\n\
             Použi tieto dáta pri odpovedi. Zhrň ich — nepísaj raw obsah celých emailov.\n\n{}",
            parts.join("\n\n")
        ))
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

async fn load_session_history(db: &Arc<Mutex<Connection>>, session_id: &str) -> Vec<Message> {
    let db = db.lock().await;
    let mut stmt = match db.prepare(
        "SELECT role, content FROM chat_turns \
         WHERE session_id = ?1 ORDER BY created_at DESC LIMIT 40"
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
