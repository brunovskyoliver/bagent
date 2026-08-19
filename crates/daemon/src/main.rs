use anyhow::{Context, Result};
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
    AgentInference, PromptBuilder, PromptTrace, ScreenIntentClassifier, SelectedSkill, TaskRater,
};
use bagent_attachments::extract as extract_attachment;
use bagent_memory::MemoryStore;
use bagent_rules::{ApprovalLevel, RuleEngine, DEFAULT_RULES_YAML};
use bagent_skills::{selector as skill_selector, LoadedSkill};
use bagentd::model_runtime::{ModelRuntime, ProductionModelConfig, RuntimePhase};
use bagentd::unified_work::UnifiedWorkAuthority;
use bagentd::work_coordinator::{
    ApprovalState, CommandError, ConversationTurnIdentity, CoordinatorConfig, CurrentChatIdentity,
    DaemonGeneration, EventCursor, EventRead, WorkCoordinator, WorkIdentity, WorkOrigin,
    WorkRecord, WorkRevision, WorkSnapshot, WorkState,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use basert_connector::{Message, DEFAULT_API_KEY, DEFAULT_BASE_URL, DEFAULT_CHAT_MODEL};
use codex_connector::{
    CodexConfig, CodexConnector, CodexContextPacket, CodexExpectedOutput, CodexTask, ContextItem,
};
use filesystem_connector::{
    self, open as fs_open, search as fs_search, FileSearchRequest, FsConnector, ReadTextRequest,
};
use odoo_connector::{OdooConfig, OdooConnector, OdooError, OdooRecordRef};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, convert::Infallible, io::Write, path::PathBuf, sync::Arc};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use whatsapp_connector::{
    WhatsappConfig, WhatsappConnectionStatus, WhatsappConnector, WhatsappSendTarget,
};

mod agent_exec;
mod automations_api;
mod evidence;
mod scheduler;

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
    /// Local opt-in rollback flag for deterministic typed Mail/web evidence routing.
    evidence_orchestrator: agent_exec::EvidenceOrchestratorFlag,
    attachments_dir: PathBuf,
    model_runtime: Arc<ModelRuntime>,
    work_authority: Arc<UnifiedWorkAuthority>,
    /// Shared preferred/fallback synthesis lifecycle for chat and automations.
    synthesis: Arc<evidence::SynthesisService>,
    /// Privacy-safe bounded structural traces for routed evidence turns.
    evidence_diagnostics: Arc<evidence::DiagnosticRecorder>,
    mail: Option<MailConnector>,
    notes: Option<NotesConnector>,
    fs: Option<FsConnector>,
    memory: Arc<MemoryStore>,
    prompt_builder: Arc<PromptBuilder>,
    rules: Arc<RuleEngine>,
    /// Loaded skill manifests + bodies, scanned at startup.
    skills: Arc<Vec<LoadedSkill>>,
    /// Deterministic task rater — classifies local vs Codex tasks.
    task_rater: Arc<TaskRater>,
    /// Codex external-reasoning connector (None when binary not found).
    codex: Option<CodexConnector>,
    /// Odoo connector — in-memory only; API key never written to disk.
    /// Swift pushes credentials from Keychain lazily when an Odoo turn needs it.
    odoo: Arc<RwLock<Option<OdooConnector>>>,
    /// Tavily key is supplied ephemerally by the signed app from Keychain.
    /// It is never persisted by the daemon or included in diagnostics.
    tavily_api_key: Arc<TavilyConfiguration>,
    /// Compiled and activated only for signed Stage 8 acceptance campaigns.
    #[cfg(feature = "stage8-acceptance")]
    acceptance: Option<evidence::AcceptanceControl>,
    /// WhatsApp Web bridge connector. Always present; owns the bridge subprocess.
    /// Bridge can autostart when a prior LocalAuth session exists, and is also
    /// controlled explicitly via `/whatsapp/start` and `/whatsapp/stop`.
    whatsapp: Arc<WhatsappConnector>,
    /// Ephemeral connector refs for current daemon run only. Never persisted.
    runtime_refs: Arc<Mutex<HashMap<String, RuntimeRefs>>>,
    /// Pinged whenever an automation is created/edited/enabled/disabled/deleted
    /// so the scheduler recomputes its next wake-up immediately.
    automations_changed: Arc<tokio::sync::Notify>,
    /// Daemon-wide event broadcast (GET /events): automation lifecycle +
    /// background approval notifications. Payloads are concise and redacted —
    /// clients refetch authoritative records.
    legacy_projection_tx: tokio::sync::broadcast::Sender<serde_json::Value>,
    /// The current Swift projection consumer. A newer snapshot claim fences
    /// every older UI process without affecting daemon-owned Work.
    ui_consumer_fence: Arc<Mutex<Option<String>>>,
}

impl AppState {
    /// Fire-and-forget daemon-wide event. Lagging/absent subscribers are fine.
    fn project_legacy_event(&self, event: serde_json::Value) {
        let _ = self.legacy_projection_tx.send(event);
    }
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
    /// Sliding-window conversation history (user/assistant turns, oldest first).
    /// Clamped server-side to 10 turns / 8k chars.
    #[serde(default)]
    history: Vec<Message>,
    model: Option<String>,
    session_id: Option<String>,
    /// IDs returned by POST /attachments — empty when no files attached.
    #[serde(default)]
    attachment_ids: Vec<String>,
    // ── OCR/selection context ────────────────────────────────────────────────
    /// On-device OCR text extracted from a temporary frame.
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
    process_id: u32,
    tavily_configuration: TavilyConfigurationStatus,
    basert: bool,
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
    if let Err(error) = state.synthesis.shutdown().await {
        tracing::error!(%error, "shutdown: Model Runtime retirement was not proven");
    }

    if let Err(e) = state.whatsapp.stop().await {
        tracing::debug!("shutdown: WhatsApp stop skipped: {e}");
    }
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
    let evidence_diagnostics = Arc::new(evidence::DiagnosticRecorder::new(
        data_dir.join("evidence-diagnostics"),
    )?);
    std::fs::write(data_dir.join("daemon.pid"), std::process::id().to_string())?;

    let db_path = data_dir.join("bagent.db");
    let pre_cutover_backup_hash = bagentd::cutover::prepare_pre_cutover_backup(
        &db_path,
        &data_dir.join("bagent.pre-stage4.sqlite"),
    )
    .map_err(|error| anyhow::anyhow!("prepare Stage 4 database backup: {error}"))?;
    let mut conn = Connection::open(&db_path)?;
    embedded::migrations::runner()
        .run(&mut conn)
        .map_err(|e| anyhow::anyhow!("migration error: {e}"))?;
    if let Some(hash) = pre_cutover_backup_hash.as_deref() {
        bagentd::cutover::record_pre_cutover_backup(&db_path, hash)
            .map_err(|error| anyhow::anyhow!("record Stage 4 backup: {error}"))?;
    }
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

    let basert_base_url =
        std::env::var("BAGENT_BASERT_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
    let basert_api_key =
        std::env::var("BAGENT_BASERT_API_KEY").unwrap_or_else(|_| DEFAULT_API_KEY.to_string());
    let default_model =
        std::env::var("BAGENT_DEFAULT_MODEL").unwrap_or_else(|_| DEFAULT_CHAT_MODEL.to_string());
    let classifier_model =
        std::env::var("BAGENT_CLASSIFIER_MODEL").unwrap_or_else(|_| DEFAULT_CHAT_MODEL.to_string());
    let synthesis_config = evidence::SynthesisConfig::from_environment();
    let model_runtime = ModelRuntime::production_from_endpoint(
        basert_base_url,
        basert_api_key.clone(),
        ProductionModelConfig::from_environment(
            default_model.clone(),
            synthesis_config.preferred_model.clone(),
            basert_api_key,
        ),
    );
    model_runtime
        .initialize()
        .await
        .context("initialize daemon-owned Model Runtime")?;
    bagentd::cutover::finalize_legacy_boundary(&db_path)
        .map_err(|error| anyhow::anyhow!("finalize Stage 4 legacy boundary: {error}"))?;
    let daemon_generation = DaemonGeneration::new(Uuid::new_v4().to_string());
    let work_authority = Arc::new(UnifiedWorkAuthority::new(
        Arc::new(
            WorkCoordinator::open(
                &db_path,
                CoordinatorConfig::default(),
                daemon_generation.clone(),
            )
            .map_err(|error| anyhow::anyhow!("open unified Work authority: {error}"))?,
        ),
        daemon_generation,
    ));
    tokio::spawn(
        work_authority
            .clone()
            .run_dispatcher(|| chrono::Utc::now().timestamp().max(0) as u64),
    );
    #[cfg(feature = "stage8-acceptance")]
    let acceptance = evidence::acceptance_runtime_enabled(
        std::env::var(evidence::STAGE8_ACCEPTANCE_FIXTURES_ENV)
            .ok()
            .as_deref(),
    )
    .then(evidence::AcceptanceControl::default);
    let synthesis = evidence::SynthesisService::new(model_runtime.clone(), synthesis_config);

    // MemoryStore uses a separate connection with std::sync::Mutex (blocking SQLite ops)
    let mem_conn = rusqlite::Connection::open(&db_path)?;
    let mem_db = Arc::new(std::sync::Mutex::new(mem_conn));
    let memory = Arc::new(MemoryStore::new(mem_db).with_data_dir(data_dir.clone()));
    let prompt_builder = Arc::new(PromptBuilder::new());

    let evidence_orchestrator = agent_exec::EvidenceOrchestratorFlag::from_local_env();
    tracing::info!(
        enabled = evidence_orchestrator == agent_exec::EvidenceOrchestratorFlag::Enabled,
        env = agent_exec::EVIDENCE_ORCHESTRATOR_FLAG_ENV,
        "typed evidence production routing"
    );

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
    let tavily_api_key = Arc::new(TavilyConfiguration::pending());

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
        db_path,
        token,
        default_model,
        debug_dir,
        classifier_model,
        evidence_orchestrator,
        attachments_dir,
        model_runtime,
        work_authority,
        synthesis,
        evidence_diagnostics,
        mail,
        notes,
        fs,
        memory,
        prompt_builder,
        rules,
        skills,
        task_rater,
        codex,
        odoo,
        tavily_api_key,
        #[cfg(feature = "stage8-acceptance")]
        acceptance,
        whatsapp,
        runtime_refs: Arc::new(Mutex::new(HashMap::new())),
        automations_changed: Arc::new(tokio::sync::Notify::new()),
        legacy_projection_tx: tokio::sync::broadcast::channel(256).0,
        ui_consumer_fence: Arc::new(Mutex::new(None)),
    };
    state.synthesis.start_maintenance().await;

    let work_projection_state = state.clone();
    tokio::spawn(async move {
        let mut cursor = work_projection_state
            .work_authority
            .coordinator()
            .snapshot()
            .map(|snapshot| snapshot.cursor)
            .unwrap_or_default();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if let Ok(bagentd::work_coordinator::EventRead::Events(events)) =
                work_projection_state.work_authority.coordinator().events(
                    Some(cursor),
                    work_projection_state.work_authority.generation(),
                )
            {
                for event in events {
                    cursor = event.event_cursor;
                    if let Ok(value) = serde_json::to_value(event) {
                        work_projection_state.project_legacy_event(value);
                    }
                }
            }
        }
    });

    // Daemon-owned automation scheduler: recovery at startup, then sleeps
    // until the next due instant (woken immediately by automations_changed).
    tokio::spawn(scheduler::run_scheduler(state.clone()));

    let shutdown_state = state.clone();
    #[allow(unused_mut)]
    let mut app = Router::new()
        .route("/health", get(health))
        .route("/events", get(events_stream))
        .route("/work/snapshot", get(work_snapshot))
        .route("/work/events", get(work_events))
        .route(
            "/work/attention/acknowledge",
            post(acknowledge_work_attention),
        )
        .route("/models", get(models))
        .route("/chat", post(chat))
        .route("/embeddings", post(embeddings))
        .route("/approvals/pending", get(approvals_pending))
        .route("/approvals/:id/decide", post(approval_decide))
        .route("/rules", get(rules_get).post(rules_save))
        // Automations — persisted scheduled agent tasks
        .route(
            "/automations",
            get(automations_api::automations_list).post(automations_api::automations_create),
        )
        .route(
            "/automations/:id",
            get(automations_api::automation_get)
                .patch(automations_api::automation_patch)
                .delete(automations_api::automation_delete),
        )
        .route(
            "/automations/:id/enable",
            post(automations_api::automation_enable),
        )
        .route(
            "/automations/:id/disable",
            post(automations_api::automation_disable),
        )
        .route(
            "/automations/:id/run-now",
            post(automations_api::automation_run_now),
        )
        .route(
            "/automations/:id/runs",
            get(automations_api::automation_runs),
        )
        .route(
            "/automations/:id/runs/:run_id",
            get(automations_api::automation_run),
        )
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
        .route(
            "/diagnostics/evidence/:turn_id/export",
            get(evidence_diagnostic_export),
        )
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
        .route("/web/tavily/config", post(tavily_config_handler))
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
        .route("/whatsapp/send", post(whatsapp_send_handler));
    #[cfg(feature = "stage8-acceptance")]
    if state.acceptance.is_some() {
        app = app.route(
            "/acceptance/stage8/fixture",
            post(stage8_acceptance_fixture_handler),
        );
    } else {
        app = app.route(
            "/acceptance/stage8/fixture",
            post(stage8_acceptance_not_found_handler),
        );
    }
    #[cfg(not(feature = "stage8-acceptance"))]
    {
        app = app.route(
            "/acceptance/stage8/fixture",
            post(stage8_acceptance_not_found_handler),
        );
    }
    let app = app
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
fn screen_intent_degraded() -> (StatusCode, Json<serde_json::Value>) {
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

async fn screen_intent_handler(
    State(state): State<AppState>,
    Json(req): Json<ScreenIntentRequest>,
) -> impl IntoResponse {
    let work_identity = match state.work_authority.submit_conversation(
        format!("screen-intent-admit:{}", Uuid::new_v4()),
        CurrentChatIdentity::new(format!("screen-intent-chat:{}", Uuid::new_v4())),
        ConversationTurnIdentity::new(format!("screen-intent-turn:{}", Uuid::new_v4())),
        chrono::Utc::now().timestamp().max(0) as u64,
    ) {
        Ok(identity) => identity,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "wants_screen": false, "wants_ocr": false, "wants_selection": false
                })),
            )
        }
    };
    // Runs detached in a spawned task so a client disconnect (axum drops this
    // handler's future on the response side) cannot cancel execution between
    // `admit` granting a capacity slot and the terminal `release_slot` — a
    // dropped future here would otherwise leak the slot forever (`admit` has
    // no timeout, and the foreground dispatch rule requires
    // `foreground_running == 0`).
    let task_state = state.clone();
    let task_work_identity = work_identity.clone();
    let task = tokio::spawn(async move {
        let state = task_state;
        let work_identity = task_work_identity;
        state.work_authority.admit(work_identity.clone()).await;
        let waiting_revision = state
            .work_authority
            .current(&work_identity)
            .ok()
            .flatten()
            .map(|record| record.revision);
        let running = waiting_revision.and_then(|revision| {
            state
                .work_authority
                .transition(
                    format!("screen-intent-running:{}", Uuid::new_v4()),
                    work_identity.clone(),
                    revision,
                    WorkState::Running,
                )
                .ok()
        });
        if running.is_none() {
            // admit() already granted this Work its execution slot; a failed
            // WaitingForModel/Running lookup or transition still owes a release.
            state.work_authority.release_slot(&work_identity);
        }
        let classifier_runtime: Arc<dyn AgentInference> =
            Arc::new(state.work_authority.model_runtime(
                state.model_runtime.clone(),
                work_identity.clone(),
                bagentd::unified_work::ExecutionOrigin::Foreground,
            ));
        let classifier =
            ScreenIntentClassifier::new(classifier_runtime, state.classifier_model.clone());
        match classifier.classify(&req.message, "").await {
            Ok(intent) => {
                if let Some(revision) = running {
                    state.work_authority.release_slot(&work_identity);
                    let _ = state.work_authority.transition(
                        format!("screen-intent-complete:{}", Uuid::new_v4()),
                        work_identity,
                        revision,
                        WorkState::Completed,
                    );
                }
                (
                    StatusCode::OK,
                    Json(serde_json::to_value(&intent).unwrap_or_default()),
                )
            }
            Err(_) => {
                if let Some(revision) = running {
                    state.work_authority.release_slot(&work_identity);
                    let _ = state.work_authority.transition(
                        format!("screen-intent-failed:{}", Uuid::new_v4()),
                        work_identity,
                        revision,
                        WorkState::Failed,
                    );
                }
                // Graceful degrade — caller treats unknown as "no screen needed"
                screen_intent_degraded()
            }
        }
    });
    match task.await {
        Ok(response) => response,
        // The spawned task already released its slot on every one of its own
        // exit paths before this join could fail (join only fails on panic,
        // never on the caller side dropping this outer future).
        Err(_) => screen_intent_degraded(),
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

async fn evidence_diagnostic_export(
    State(state): State<AppState>,
    Path(turn_id): Path<String>,
) -> impl IntoResponse {
    match state.evidence_diagnostics.export(&turn_id) {
        Ok(trace) => (StatusCode::OK, Json(trace)).into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "evidence trace not found"})),
        )
            .into_response(),
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
    if req.uri().path() == "/acceptance/stage8/fixture" && !stage8_acceptance_route_enabled(&state)
    {
        return Ok(next.run(req).await);
    }
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

fn stage8_acceptance_route_enabled(state: &AppState) -> bool {
    #[cfg(feature = "stage8-acceptance")]
    {
        state.acceptance.is_some()
    }
    #[cfg(not(feature = "stage8-acceptance"))]
    {
        let _ = state;
        false
    }
}

async fn stage8_acceptance_not_found_handler() -> StatusCode {
    StatusCode::NOT_FOUND
}

// ── Core handlers ─────────────────────────────────────────────────────────────

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let odoo_configured = state.odoo.read().await.is_some();
    let tavily_configuration = state.tavily_api_key.status();
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
        process_id: std::process::id(),
        tavily_configuration,
        basert: state.model_runtime.health().await,
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

#[derive(Deserialize)]
struct WorkSnapshotQuery {
    consumer_fence: String,
}

#[derive(Deserialize)]
struct WorkEventsQuery {
    after: u64,
    daemon_generation: String,
    consumer_fence: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcknowledgeWorkAttentionRequest {
    command_identity: String,
    consumer_fence: String,
    work_identity: String,
    expected_revision: u64,
}

async fn acknowledge_work_attention(
    State(state): State<AppState>,
    Json(request): Json<AcknowledgeWorkAttentionRequest>,
) -> impl IntoResponse {
    let fence_matches = state
        .ui_consumer_fence
        .lock()
        .await
        .as_deref()
        .is_some_and(|active| active == request.consumer_fence);
    if !fence_matches {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "code": "stale_consumer_fence",
                "error": "stale consumer fence"
            })),
        );
    }
    match state.work_authority.acknowledge_attention(
        request.command_identity,
        WorkIdentity::new(request.work_identity),
        WorkRevision::new(request.expected_revision),
    ) {
        Ok(revision) => (
            StatusCode::OK,
            Json(serde_json::json!({ "revision": revision.value() })),
        ),
        Err(error) => acknowledge_attention_error_response(error),
    }
}

fn acknowledge_attention_error_response(
    error: CommandError,
) -> (StatusCode, Json<serde_json::Value>) {
    match error {
        error @ (CommandError::Conflict { .. } | CommandError::TerminalTarget) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "code": "work_conflict",
                "error": format!("{error}")
            })),
        ),
        error => {
            tracing::error!(%error, "failed to acknowledge Work attention");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "code": "internal_error",
                    "error": "failed to acknowledge Work attention"
                })),
            )
        }
    }
}

async fn work_snapshot(
    State(state): State<AppState>,
    Query(query): Query<WorkSnapshotQuery>,
) -> impl IntoResponse {
    if query.consumer_fence.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "missing consumer fence" })),
        );
    }
    *state.ui_consumer_fence.lock().await = Some(query.consumer_fence);
    match authoritative_notch_snapshot(&state) {
        Ok((snapshot, context)) => (
            StatusCode::OK,
            Json(notch_snapshot_value(&snapshot, &context)),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{error}") })),
        ),
    }
}

async fn work_events(
    State(state): State<AppState>,
    Query(query): Query<WorkEventsQuery>,
) -> impl IntoResponse {
    let fence_matches = state
        .ui_consumer_fence
        .lock()
        .await
        .as_deref()
        .is_some_and(|active| active == query.consumer_fence);
    if !fence_matches {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "stale consumer fence" })),
        );
    }

    let coordinator = state.work_authority.coordinator();
    match coordinator.notch_events(
        EventCursor::new(query.after),
        &DaemonGeneration::new(query.daemon_generation),
    ) {
        Ok(EventRead::Gap { .. }) => match authoritative_notch_snapshot(&state) {
            Ok((snapshot, context)) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "kind": "gap",
                    "snapshot": notch_snapshot_value(&snapshot, &context),
                })),
            ),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("{error}") })),
            ),
        },
        Ok(EventRead::Events(events)) if events.is_empty() => (
            StatusCode::OK,
            Json(serde_json::json!({ "kind": "events", "events": [] })),
        ),
        Ok(EventRead::Events(events)) => match authoritative_notch_snapshot(&state) {
            Ok((snapshot, context)) => {
                if event_batch_requires_snapshot(&events, &snapshot) {
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "kind": "gap",
                            "snapshot": notch_snapshot_value(&snapshot, &context),
                        })),
                    );
                }
                let projected = events
                    .iter()
                    .filter_map(|event| {
                        snapshot
                            .works
                            .iter()
                            .enumerate()
                            .find(|(_, work)| work.identity == event.work_identity)
                            .map(|(index, work)| {
                                let mut value = notch_work_value(work, index as u64, &context);
                                let object = value.as_object_mut().expect("work projection object");
                                object.insert(
                                    "revision".to_owned(),
                                    serde_json::json!(event.work_revision.value()),
                                );
                                object.insert(
                                    "state".to_owned(),
                                    serde_json::to_value(event.state)
                                        .expect("serializable Work state"),
                                );
                                object.insert(
                                    "activity".to_owned(),
                                    event
                                        .activity
                                        .map(|category| serde_json::json!({ "category": category }))
                                        .unwrap_or(serde_json::Value::Null),
                                );
                                serde_json::json!({
                                    "schemaVersion": event.schema_version,
                                    "cursor": event.event_cursor.value(),
                                    "daemonGeneration": event.daemon_generation.as_str(),
                                    "work": value,
                                    "pendingApprovals": notch_pending_approvals_value(&snapshot),
                                    "model": context.model_phase,
                                })
                            })
                    })
                    .collect::<Vec<_>>();
                (
                    StatusCode::OK,
                    Json(serde_json::json!({ "kind": "events", "events": projected })),
                )
            }
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("{error}") })),
            ),
        },
        Err(error) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": format!("{error}") })),
        ),
    }
}

fn event_batch_requires_snapshot(
    events: &[bagentd::work_coordinator::WorkEvent],
    snapshot: &WorkSnapshot,
) -> bool {
    events.len() != 1
        || events[0].event_cursor.value() != snapshot.cursor.value()
        || !snapshot
            .works
            .iter()
            .any(|work| work.identity == events[0].work_identity)
}

struct NotchProjectionContext {
    model_phase: &'static str,
    automation_names: HashMap<String, String>,
    automation_definition_identities: HashMap<String, String>,
    automation_session_identities: HashMap<String, String>,
    terminal_attention: HashMap<String, &'static str>,
    terminal_orders: HashMap<String, u64>,
    queue_positions: HashMap<String, u64>,
    claimed_orders: HashMap<String, u64>,
}

fn authoritative_notch_snapshot(
    state: &AppState,
) -> Result<(WorkSnapshot, NotchProjectionContext)> {
    let model_phase = match state.model_runtime.snapshot().phase {
        RuntimePhase::Unavailable => "unavailable",
        RuntimePhase::Unloaded => "unloaded",
        RuntimePhase::Loading(_) => "loading",
        RuntimePhase::LoadedNotReady(_) => "loaded_not_ready",
        RuntimePhase::Ready(_) => "ready",
        RuntimePhase::Retiring(_) => "retiring",
        RuntimePhase::Poisoned(_) => "poisoned",
        RuntimePhase::Restarting => "restarting",
    };
    state
        .work_authority
        .coordinator()
        .projected_snapshot(|connection, snapshot| {
            notch_projection_context(connection, snapshot, model_phase)
                .map_err(|error| CommandError::Storage(error.to_string()))
        })
        .map_err(Into::into)
}

const NOTCH_PROJECTION_CONTEXT_SQL: &str =
    "SELECT w.identity, w.rowid, a.name, r.historical_automation_identity,
            r.automation_session_identity, s.attention_state, w.state, w.updated_at
     FROM json_each(?1) projected
     JOIN works w ON w.identity = projected.value
     LEFT JOIN work_automation_runs r ON r.work_identity = w.identity
     LEFT JOIN automations a ON a.id = r.historical_automation_identity
     LEFT JOIN work_automation_sessions s
       ON s.automation_session_identity = r.automation_session_identity
     ORDER BY w.identity ASC";

fn notch_projection_context(
    db: &Connection,
    snapshot: &WorkSnapshot,
    model_phase: &'static str,
) -> Result<NotchProjectionContext> {
    let mut automation_names = HashMap::new();
    let mut automation_definition_identities = HashMap::new();
    let mut automation_session_identities = HashMap::new();
    let mut terminal_attention = HashMap::new();
    let mut terminal_orders = HashMap::new();
    let mut queue_positions = HashMap::new();
    let mut claimed_orders = HashMap::new();
    let identities = snapshot
        .works
        .iter()
        .map(|work| work.identity.as_str())
        .collect::<Vec<_>>();
    let projected_identities = serde_json::to_string(&identities)?;
    let mut statement = db.prepare(NOTCH_PROJECTION_CONTEXT_SQL)?;
    let rows = statement.query_map(rusqlite::params![projected_identities], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, u64>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;
    let mut terminal_candidates = Vec::new();
    for row in rows {
        let (
            identity,
            claimed_order,
            automation_name,
            definition_identity,
            session_identity,
            attention_state,
            state,
            updated_at,
        ) = row?;
        claimed_orders.insert(identity.clone(), claimed_order);
        if let Some(name) = automation_name {
            automation_names.insert(identity.clone(), name);
        }
        if let Some(definition_identity) = definition_identity {
            automation_definition_identities.insert(identity.clone(), definition_identity);
        }
        if let Some(session_identity) = session_identity {
            automation_session_identities.insert(identity.clone(), session_identity);
        }
        if attention_state.as_deref() == Some("unread") {
            let attention = match state.as_str() {
                "failed" => Some("failed"),
                "partial" => Some("partial"),
                "completed" => Some("unread"),
                _ => None,
            };
            if let Some(attention) = attention {
                terminal_attention.insert(identity.clone(), attention);
            }
        }
        if matches!(state.as_str(), "completed" | "partial" | "failed") {
            terminal_candidates.push((updated_at, identity));
        }
    }
    terminal_candidates.sort();
    let mut terminal_order = 0;
    let mut previous_timestamp = None;
    for (timestamp, identity) in terminal_candidates {
        if previous_timestamp.as_deref() != Some(timestamp.as_str()) {
            terminal_order += 1;
            previous_timestamp = Some(timestamp);
        }
        terminal_orders.insert(identity, terminal_order);
    }
    let mut queued_automations = snapshot
        .works
        .iter()
        .filter(|work| {
            work.state == WorkState::Queued && matches!(work.origin, WorkOrigin::Automation { .. })
        })
        .map(|work| {
            (
                claimed_orders
                    .get(work.identity.as_str())
                    .copied()
                    .unwrap_or(u64::MAX),
                work.identity.as_str().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    queued_automations.sort_by_key(|(claimed_order, _)| *claimed_order);
    for (index, (_, identity)) in queued_automations.into_iter().enumerate() {
        queue_positions.insert(identity, index as u64 + 1);
    }
    Ok(NotchProjectionContext {
        model_phase,
        automation_names,
        automation_definition_identities,
        automation_session_identities,
        terminal_attention,
        terminal_orders,
        queue_positions,
        claimed_orders,
    })
}

fn notch_snapshot_value(
    snapshot: &WorkSnapshot,
    context: &NotchProjectionContext,
) -> serde_json::Value {
    let works = snapshot
        .works
        .iter()
        .enumerate()
        .filter(|(_, work)| context.claimed_orders.contains_key(work.identity.as_str()))
        .map(|(index, work)| notch_work_value(work, index as u64, context))
        .collect::<Vec<_>>();
    let pending_approvals = notch_pending_approvals_value(snapshot);
    serde_json::json!({
        "schemaVersion": snapshot.schema_version,
        "cursor": snapshot.cursor.value(),
        "daemonGeneration": snapshot.daemon_generation.as_str(),
        "works": works,
        "pendingApprovals": pending_approvals,
        "model": context.model_phase,
    })
}

fn notch_pending_approvals_value(snapshot: &WorkSnapshot) -> serde_json::Value {
    serde_json::Value::Array(
        snapshot
            .approvals
            .iter()
            .filter(|approval| approval.state == ApprovalState::Pending)
            .map(|approval| {
                serde_json::json!({
                    "identity": approval.identity.as_str(),
                    "workIdentity": approval.work_identity.as_str(),
                })
            })
            .collect::<Vec<_>>(),
    )
}

fn notch_work_value(
    work: &WorkRecord,
    claimed_order: u64,
    context: &NotchProjectionContext,
) -> serde_json::Value {
    let origin = match &work.origin {
        WorkOrigin::Conversation { .. } => "conversation",
        WorkOrigin::Automation { .. } => "automation",
    };
    serde_json::json!({
        "identity": work.identity.as_str(),
        "revision": work.revision.value(),
        "origin": origin,
        "state": work.state,
        "activity": work.activity.map(|category| serde_json::json!({ "category": category })),
        "queuePosition": context.queue_positions.get(work.identity.as_str()),
        "automationDisplayName": context.automation_names.get(work.identity.as_str()),
        "automationDefinitionIdentity": context.automation_definition_identities.get(work.identity.as_str()),
        "automationSessionIdentity": context.automation_session_identities.get(work.identity.as_str()),
        "terminalAttention": context.terminal_attention.get(work.identity.as_str()),
        "terminalOrder": context.terminal_orders.get(work.identity.as_str()),
        "claimedOrder": context.claimed_orders.get(work.identity.as_str()).copied().unwrap_or(claimed_order),
    })
}

/// Daemon-wide SSE stream: automation lifecycle + approval notifications.
/// Typed envelopes only — clients refetch authoritative records on receipt.
async fn events_stream(
    State(state): State<AppState>,
) -> Sse<ReceiverStream<Result<Event, Infallible>>> {
    let mut sub = state.legacy_projection_tx.subscribe();
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(64);
    tokio::spawn(async move {
        loop {
            match sub.recv().await {
                Ok(v) => {
                    if tx
                        .send(Ok(Event::default().data(v.to_string())))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

async fn models(State(state): State<AppState>) -> impl IntoResponse {
    match state.model_runtime.models().await {
        Ok(models) => {
            let chat_models = models
                .into_iter()
                .map(|model| model.id)
                .filter(|name| name == &state.default_model)
                .collect::<Vec<_>>();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "models": chat_models })),
            )
        }
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
            "SELECT id, tool_name, description, expires_at, created_at, origin_json \
             FROM pending_approvals \
             WHERE decision IS NULL AND expires_at > datetime('now') \
             ORDER BY created_at",
        )
        .ok()
        .and_then(|mut s| {
            s.query_map([], |row| {
                let origin: Option<String> = row.get(5)?;
                Ok(serde_json::json!({
                    "id":          row.get::<_, String>(0)?,
                    "tool_name":   row.get::<_, String>(1)?,
                    "description": row.get::<_, Option<String>>(2)?,
                    "expires_at":  row.get::<_, String>(3)?,
                    "created_at":  row.get::<_, String>(4)?,
                    "origin":      origin
                        .and_then(|o| serde_json::from_str::<serde_json::Value>(&o).ok()),
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
    let decision = if req.allow { "allow" } else { "deny" };
    let decided_at = chrono::Utc::now().to_rfc3339();
    // Read both the compatibility projection and canonical approval before
    // mutating either.  When this is a Work approval, the coordinator commit
    // must win first: a failed/stale command must never leave Swift observing
    // an accepted legacy decision that the authoritative state machine
    // rejected.
    let canonical = {
        let db = state.db.lock().await;
        db.query_row(
            "SELECT a.work_identity, w.revision, w.origin_kind
             FROM pending_approvals p
             LEFT JOIN work_approvals a ON a.identity=p.id AND a.state='pending'
             LEFT JOIN works w ON w.identity=a.work_identity
             WHERE p.id=?1 AND p.decision IS NULL AND p.expires_at > ?2",
            rusqlite::params![id, decided_at],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<u64>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .ok()
    };
    let Some((work, revision, origin_kind)) = canonical else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "approval not found or already decided" })),
        );
    };
    if let (Some(work), Some(revision), Some(origin_kind)) = (work, revision, origin_kind) {
        let execution_origin = if origin_kind == "automation" {
            bagentd::unified_work::ExecutionOrigin::Automation
        } else {
            bagentd::unified_work::ExecutionOrigin::Foreground
        };
        let work_identity = bagentd::work_coordinator::WorkIdentity::new(work);
        match state.work_authority.resolve_approval(
            format!("approval-decision:{id}"),
            work_identity.clone(),
            WorkRevision::new(revision),
            bagentd::work_coordinator::ApprovalIdentity::new(id.clone()),
            req.allow,
            0,
        ) {
            Ok(_) => {
                state
                    .work_authority
                    .resume(work_identity, execution_origin)
                    .await
            }
            Err(error) => {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": error.to_string() })),
                );
            }
        }
    }
    let changed = {
        let db = state.db.lock().await;
        db.execute(
            "UPDATE pending_approvals SET decision = ?1, decided_at = ?2
             WHERE id = ?3 AND decision IS NULL AND expires_at > ?2",
            rusqlite::params![decision, decided_at, id],
        )
        .unwrap_or(0)
    };
    if changed == 1 {
        let db = state.db.lock().await;
        let _ = db.execute(
            "INSERT INTO audit_entries (action, payload, model) VALUES ('approval_decide', ?1, '')",
            rusqlite::params![serde_json::json!({"id": id, "decision": decision}).to_string()],
        );
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
    State(_state): State<AppState>,
    Json(_req): Json<serde_json::Value>,
) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": {
                "code": "embeddings_disabled",
                "message": "Semantic embeddings are disabled; bagent uses full-text retrieval."
            }
        })),
    )
}

async fn chat(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Sse<ReceiverStream<Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel(64);
    let model = req.model.clone().unwrap_or(state.default_model.clone());
    let db = state.db.clone();
    let user_message = req.message.clone();
    let prompt_builder = state.prompt_builder.clone();
    let debug_dir = state.debug_dir.clone();
    let attachment_ids = req.attachment_ids.clone();
    // Screen context is text-only: screenshot bytes are intentionally ignored.
    let screen_ocr_text = req.screen_ocr_text.clone();
    let active_app = req.active_app.clone();
    let selected_text = req.selected_text.clone();
    let source_mode = req.source_mode.clone();
    let skills = state.skills.clone();
    let task_rater = state.task_rater.clone();
    let runtime_refs = state.runtime_refs.clone();
    #[cfg(feature = "stage8-acceptance")]
    let acceptance_runtime_active = state.acceptance.is_some();
    #[cfg(not(feature = "stage8-acceptance"))]
    let acceptance_runtime_active = false;
    #[cfg(feature = "stage8-acceptance")]
    let acceptance_fixture_active = state
        .acceptance
        .as_ref()
        .is_some_and(|control| control.selection().is_some());
    #[cfg(not(feature = "stage8-acceptance"))]
    let acceptance_fixture_active = false;

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

        // Sliding-window conversation history supplied by the client so
        // follow-ups ("what other options do I have?") resolve. Clamped here —
        // the client is trusted UI but the caps are the contract.
        const HISTORY_MAX_TURNS: usize = 10;
        const HISTORY_MAX_TURN_CHARS: usize = 1_500;
        const HISTORY_MAX_TOTAL_CHARS: usize = 8_000;
        let mut history: Vec<Message> = req
            .history
            .iter()
            .rev()
            .filter(|m| (m.role == "user" || m.role == "assistant") && !m.content.trim().is_empty())
            .take(HISTORY_MAX_TURNS)
            .map(|m| Message {
                role: m.role.clone(),
                content: m.content.chars().take(HISTORY_MAX_TURN_CHARS).collect(),
                ..Message::user("")
            })
            .collect();
        history.reverse();
        while history.iter().map(|m| m.content.len()).sum::<usize>() > HISTORY_MAX_TOTAL_CHARS {
            history.remove(0);
        }
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

        // Load text attachment context. Image inference is intentionally unsupported.
        struct AttachmentData {
            ctx: Option<String>,
            turn_ids: Vec<String>,
            has_unsupported_image: bool,
        }
        let att_data = {
            let mut ctx_parts: Vec<String> = Vec::new();
            let mut turn_ids: Vec<String> = Vec::new();
            let mut has_unsupported_image = false;

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
                        if let Ok((filename, kind, _bytes_path, _mime, extracted_text)) = row {
                            turn_ids.push(att_id.clone());
                            if kind == "image" {
                                has_unsupported_image = true;
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

            AttachmentData {
                ctx,
                turn_ids,
                has_unsupported_image,
            }
        };

        if att_data.has_unsupported_image {
            let _ = tx
                .send(Ok(Event::default().data(
                    serde_json::json!({
                        "type": "error",
                        "code": "image_unsupported",
                        "message": "Image attachments are not supported by the configured text-only BaseRT model."
                    })
                    .to_string(),
                )))
                .await;
            return;
        }

        // Screen OCR/selection is in-memory only and screenshot bytes are discarded.
        let att_data = {
            let AttachmentData {
                ctx,
                turn_ids,
                has_unsupported_image,
            } = att_data;

            let mut screen_ctx_parts: Vec<String> = Vec::new();
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

            AttachmentData {
                ctx: merged_ctx,
                turn_ids,
                has_unsupported_image,
            }
        };

        let effective_model = model.clone();

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
            let bagent_selected = skill_selector::select(&[], &skills, &user_message);
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
        let messages = match prompt_builder
            .build(
                &user_message,
                lang,
                &bagent_agent::ResponseLanguageHint::MatchUser,
                &selected_skills,
                &selected_memory,
                &corrections,
                tool_ctx,
                att_data.ctx,
                history.clone(),
                session_summary,
                recall_candidates,
                false,
                None,
                &user_message,
            )
            .await
        {
            Ok(mut built) => {
                // History goes between system layers and the current user turn.
                // PromptBuilder stays stateless; the window is spliced here.
                if !history.is_empty() {
                    let hist_chars: usize = history.iter().map(|m| m.content.len()).sum();
                    built.trace.layers.push(bagent_agent::PromptLayerTrace {
                        name: "conversation_history".to_string(),
                        role: "user/assistant".to_string(),
                        included: true,
                        chars: hist_chars,
                        preview: preview_text(
                            &history
                                .last()
                                .map(|m| m.content.clone())
                                .unwrap_or_default(),
                            240,
                        ),
                    });
                    built.messages.extend(history.clone());
                }
                built.messages.push(Message::user(&user_message));
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
                let mut msgs = history.clone();
                msgs.push(Message::user(&user_message));
                msgs
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

        // ── Agentic tool loop (shared execution service) ─────────────────────
        // One loop for chat and automations lives in agent_exec. Guardrails
        // live in its dispatcher: rules engine verdicts on the actual args,
        let work_identity = match state.work_authority.submit_conversation(
            format!("chat-admit:{}", Uuid::new_v4()),
            CurrentChatIdentity::new(session_id.clone()),
            ConversationTurnIdentity::new(Uuid::new_v4().to_string()),
            chrono::Utc::now().timestamp().max(0) as u64,
        ) {
            Ok(identity) => identity,
            Err(_) => return,
        };
        state.work_authority.admit(work_identity.clone()).await;
        let waiting_revision = match state
            .work_authority
            .current(&work_identity)
            .ok()
            .flatten()
            .map(|record| record.revision)
        {
            Some(revision) => revision,
            None => {
                state.work_authority.release_slot(&work_identity);
                return;
            }
        };
        let running_revision = match state.work_authority.transition(
            format!("chat-running:{}", Uuid::new_v4()),
            work_identity.clone(),
            waiting_revision,
            WorkState::Running,
        ) {
            Ok(revision) => revision,
            Err(_) => {
                state.work_authority.release_slot(&work_identity);
                return;
            }
        };

        // PathPolicy (inside the fs connector), approval modal for writes,
        // per-turn budgets, and an audit entry per call.
        let tools = agent_exec::build_tools(&state, false).await;

        // Forward execution events onto this request's SSE stream.
        let (ev_tx, mut ev_rx) = mpsc::channel::<serde_json::Value>(64);
        let sink = if acceptance_fixture_active {
            agent_exec::EventSink::without_diagnostics(ev_tx)
        } else {
            agent_exec::EventSink::with_diagnostics(ev_tx, state.evidence_diagnostics.clone())
        };
        let sse_tx = tx.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(v) = ev_rx.recv().await {
                if sse_tx
                    .send(Ok(Event::default().data(v.to_string())))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let loop_result = agent_exec::run_agent_loop(
            &state,
            &sink,
            &agent_exec::ExecOrigin::Chat,
            work_identity.clone(),
            &session_id,
            &effective_model,
            messages,
            tools,
        )
        .await;
        drop(sink);
        let _ = forwarder.await;
        let full_response = match loop_result {
            Ok(outcome) => {
                let terminal_revision = state
                    .work_authority
                    .current(&work_identity)
                    .ok()
                    .flatten()
                    .map(|record| record.revision)
                    .unwrap_or(running_revision);
                state.work_authority.release_slot(&work_identity);
                let _ = state.work_authority.transition(
                    format!("chat-complete:{}", Uuid::new_v4()),
                    work_identity,
                    terminal_revision,
                    WorkState::Completed,
                );
                outcome.final_text
            }
            // Error already emitted to the stream / client gone.
            Err(_) => {
                let terminal_revision = state
                    .work_authority
                    .current(&work_identity)
                    .ok()
                    .flatten()
                    .map(|record| record.revision)
                    .unwrap_or(running_revision);
                state.work_authority.release_slot(&work_identity);
                let _ = state.work_authority.transition(
                    format!("chat-failed:{}", Uuid::new_v4()),
                    work_identity,
                    terminal_revision,
                    WorkState::Failed,
                );
                return;
            }
        };

        let response_for_audit = full_response.clone();
        if let Ok(db) = db.try_lock() {
            let _ = db.execute(
                "INSERT INTO audit_entries (action, payload, model) VALUES (?1, ?2, ?3)",
                rusqlite::params!["chat", &user_message, &effective_model],
            );
        }

        if !acceptance_runtime_active {
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
                            images_count: 0,
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

    let hydrated = match mail.hydrate_message(rowid).await {
        Ok(Some(hydrated)) => hydrated,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "message not found" })),
            )
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };
    let state = hydrated.state;
    let used_automation = hydrated.used_automation;
    let msg = hydrated.message;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "message": msg,
            "body_hydration": state,
            "body_hydrated_via_automation": used_automation,
            "pii": true
        })),
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

fn attachment_mime_is_supported(mime: &str) -> bool {
    !mime.starts_with("image/")
}

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

    if !attachment_mime_is_supported(&mime) {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(serde_json::json!({
                "error": {
                    "code": "image_unsupported",
                    "message": "Image attachments are not supported by the configured text-only BaseRT model."
                }
            })),
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

/// Core approval logic: persist one authoritative request, then observe its
/// durable decision until the user decides or the 60 s deadline wins.
///
/// `sse_tx` — pass `Some(&tx)` from the chat SSE flow to emit the
/// `approval_requested` event; pass `None` for REST callers (the Swift app's
/// 1 s poll of `GET /approvals/pending` will surface the row automatically).
async fn request_approval_core(
    state: &AppState,
    tool_name: &str,
    description: &str,
    sink: Option<&agent_exec::EventSink>,
    origin_json: Option<String>,
    work_context: Option<(CanonicalApprovalWork, agent_exec::ExecOrigin)>,
) -> bool {
    let db = &state.db;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();

    let canonical_approval = work_context.as_ref().and_then(|(context, origin)| {
        let record = state
            .work_authority
            .current(&context.work_identity)
            .ok()
            .flatten()?;
        let execution_origin = if origin.unattended() {
            bagentd::unified_work::ExecutionOrigin::Automation
        } else {
            bagentd::unified_work::ExecutionOrigin::Foreground
        };
        state
            .work_authority
            .request_approval(
                format!("approval-request:{id}"),
                context.work_identity.clone(),
                record.revision,
                bagentd::work_coordinator::ApprovalIdentity::new(id.clone()),
                tool_name,
            )
            .ok()
            .map(|revision| (context.work_identity.clone(), revision, execution_origin))
    });

    if let Ok(db) = db.try_lock() {
        let _ = db.execute(
            "INSERT INTO pending_approvals (id, tool_name, description, expires_at, created_at, origin_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![id, tool_name, description, expires_at, now, origin_json],
        );
    }

    let approval_event = serde_json::json!({
        "type":        "approval_requested",
        "id":          id,
        "tool":        tool_name,
        "description": description,
        "expires_in":  60,
        "origin":      origin_json
            .as_deref()
            .and_then(|o| serde_json::from_str::<serde_json::Value>(o).ok()),
    });
    // Chat streams get it inline; the daemon-wide broadcast reaches the app
    // even when no chat stream is open (background automation approvals).
    if let Some(s) = sink {
        let _ = s.emit(approval_event.clone()).await;
    }
    state.project_legacy_event(approval_event);

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(60);
    loop {
        let decision = {
            let db = state.db.lock().await;
            db.query_row(
                "SELECT decision FROM pending_approvals WHERE id=?1",
                rusqlite::params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
        };
        if let Some(decision) = decision {
            if let Some((work, revision, execution_origin)) = canonical_approval.as_ref() {
                if state
                    .work_authority
                    .current(work)
                    .ok()
                    .flatten()
                    .is_some_and(|record| record.state == WorkState::WaitingForApproval)
                {
                    let resolved = state.work_authority.resolve_approval(
                        format!("approval-observed:{id}"),
                        work.clone(),
                        *revision,
                        bagentd::work_coordinator::ApprovalIdentity::new(id.clone()),
                        decision == "allow",
                        0,
                    );
                    if resolved.is_ok() {
                        state
                            .work_authority
                            .resume(work.clone(), *execution_origin)
                            .await;
                    }
                }
            }
            return decision == "allow";
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    let now2 = chrono::Utc::now().to_rfc3339();
    let db = state.db.lock().await;
    let _ = db.execute(
        "UPDATE pending_approvals SET decision='deny', decided_at=?1 WHERE id=?2 AND decision IS NULL",
        rusqlite::params![now2, id],
    );
    let _ = db.execute(
        "INSERT INTO audit_entries (action, payload, model) VALUES ('approval_timeout', ?1, '')",
        rusqlite::params![serde_json::json!({"id": id, "tool": tool_name}).to_string()],
    );
    drop(db);
    if let Some((work, revision, execution_origin)) = canonical_approval {
        let resolved = state.work_authority.resolve_approval(
            format!("approval-expired:{id}"),
            work.clone(),
            revision,
            bagentd::work_coordinator::ApprovalIdentity::new(id),
            false,
            0,
        );
        if resolved.is_ok() {
            state.work_authority.resume(work, execution_origin).await;
        }
    }
    false
}

#[derive(Clone)]
struct CanonicalApprovalWork {
    work_identity: bagentd::work_coordinator::WorkIdentity,
}

/// Convenience wrapper for streaming execution paths (always emits the event).
async fn request_tool_approval(
    state: &AppState,
    sink: &agent_exec::EventSink,
    origin: &agent_exec::ExecOrigin,
    work_identity: &bagentd::work_coordinator::WorkIdentity,
    tool_name: &str,
    description: &str,
) -> bool {
    request_approval_core(
        state,
        tool_name,
        description,
        Some(sink),
        origin.provenance_json(),
        Some((
            CanonicalApprovalWork {
                work_identity: work_identity.clone(),
            },
            origin.clone(),
        )),
    )
    .await
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
        &state,
        "codex.run_task",
        &approval_description,
        None, // REST path — Swift polls GET /approvals/pending
        None,
        None,
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

#[derive(Deserialize)]
struct TavilyConfigRequest {
    api_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TavilyConfigurationStatus {
    Pending,
    Absent,
    Configured,
}

struct TavilyConfiguration {
    state: std::sync::RwLock<TavilyConfigurationState>,
}

struct TavilyConfigurationState {
    credential: Option<String>,
    status: TavilyConfigurationStatus,
}

impl TavilyConfiguration {
    fn pending() -> Self {
        Self {
            state: std::sync::RwLock::new(TavilyConfigurationState {
                credential: None,
                status: TavilyConfigurationStatus::Pending,
            }),
        }
    }

    fn status(&self) -> TavilyConfigurationStatus {
        self.state
            .read()
            .expect("Tavily configuration lock poisoned")
            .status
    }

    fn credential(&self) -> Option<String> {
        self.state
            .read()
            .expect("Tavily configuration lock poisoned")
            .credential
            .clone()
    }

    async fn read(&self) -> Option<String> {
        self.credential()
    }

    fn apply(&self, credential: Option<String>) -> Result<TavilyConfigurationStatus, ()> {
        let credential = credential.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        });
        if credential.as_ref().is_some_and(|value| value.len() > 512) {
            return Err(());
        }

        let status = if credential.is_some() {
            TavilyConfigurationStatus::Configured
        } else {
            TavilyConfigurationStatus::Absent
        };
        *self
            .state
            .write()
            .expect("Tavily configuration lock poisoned") =
            TavilyConfigurationState { credential, status };
        Ok(status)
    }
}

#[cfg(feature = "stage8-acceptance")]
#[derive(Deserialize)]
struct Stage8AcceptanceFixtureRequest {
    selection: Option<evidence::AcceptanceFixtureSelection>,
}

/// Acceptance-only authenticated control. Both the compile-time feature and
/// exact runtime environment flag are required before the route is registered.
#[cfg(feature = "stage8-acceptance")]
async fn stage8_acceptance_fixture_handler(
    State(state): State<AppState>,
    Json(body): Json<Stage8AcceptanceFixtureRequest>,
) -> impl IntoResponse {
    let Some(control) = state.acceptance.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "ok": false, "error": "not_found" })),
        );
    };
    control.set(body.selection);
    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

/// Receives the Tavily credential from the signed app's Keychain and keeps it
/// only for this daemon process lifetime.
async fn tavily_config_handler(
    State(state): State<AppState>,
    Json(body): Json<TavilyConfigRequest>,
) -> impl IntoResponse {
    match state.tavily_api_key.apply(body.api_key) {
        Ok(status) => (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "status": status })),
        ),
        Err(()) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "invalid_api_key" })),
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
        &state,
        "whatsapp.send_message",
        &audit_description, // stored in audit_entries — redacted (trap #2)
        None,               // REST path; no SSE channel
        None,
        None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledgement_errors_distinguish_authoritative_conflicts_from_server_failures() {
        for error in [
            CommandError::Conflict {
                current_revision: Some(WorkRevision::new(7)),
            },
            CommandError::TerminalTarget,
        ] {
            let (status, Json(body)) = acknowledge_attention_error_response(error);
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(body["code"], "work_conflict");
        }

        for error in [
            CommandError::Storage("disk unavailable".to_owned()),
            CommandError::CorruptState("bad state".to_owned()),
            CommandError::InjectedFailure(
                bagentd::work_coordinator::FailurePoint::BeforeTransaction,
            ),
        ] {
            let (status, Json(body)) = acknowledge_attention_error_response(error);
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(body["code"], "internal_error");
            assert_eq!(body["error"], "failed to acknowledge Work attention");
            assert!(!body.to_string().contains("disk unavailable"));
        }
    }

    #[test]
    fn notch_projection_context_uses_one_structured_identity_parameter() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE works (
                 identity TEXT PRIMARY KEY,
                 state TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE TABLE work_automation_runs (
                 work_identity TEXT,
                 historical_automation_identity TEXT,
                 automation_session_identity TEXT
             );
             CREATE TABLE automations (id TEXT PRIMARY KEY, name TEXT);
             CREATE TABLE work_automation_sessions (
                 automation_session_identity TEXT,
                 attention_state TEXT
             );",
        )
        .unwrap();

        let statement = db.prepare(NOTCH_PROJECTION_CONTEXT_SQL).unwrap();
        assert_eq!(statement.parameter_count(), 1);
    }

    #[test]
    fn retained_transition_batches_replace_with_one_current_snapshot() {
        let snapshot = WorkSnapshot {
            schema_version: 1,
            cursor: EventCursor::new(22),
            daemon_generation: DaemonGeneration::new("daemon-a"),
            works: vec![WorkRecord {
                identity: WorkIdentity::new("work-a"),
                origin: WorkOrigin::Conversation {
                    current_chat_identity: CurrentChatIdentity::new("chat-a"),
                    conversation_turn_identity: ConversationTurnIdentity::new("turn-a"),
                },
                state: WorkState::Running,
                revision: WorkRevision::new(3),
                activity: None,
            }],
            automation_runs: vec![],
            approvals: vec![],
            interruptions: vec![],
            model_runtime_generation: None,
            model_runtime_trusted: true,
        };
        let transition = |cursor, state| bagentd::work_coordinator::WorkEvent {
            schema_version: 1,
            event_cursor: EventCursor::new(cursor),
            daemon_generation: DaemonGeneration::new("daemon-a"),
            committed_at: "2026-08-19T00:00:00Z".to_owned(),
            event_kind: bagentd::work_coordinator::EventKind::WorkStateChanged,
            work_identity: WorkIdentity::new("work-a"),
            work_revision: WorkRevision::new(cursor - 19),
            state,
            activity: None,
        };

        let approval_requested_then_resolved = vec![
            transition(21, WorkState::WaitingForApproval),
            transition(22, WorkState::Running),
        ];
        let completed_then_acknowledged = vec![
            transition(21, WorkState::Completed),
            transition(22, WorkState::Completed),
        ];

        assert!(event_batch_requires_snapshot(
            &approval_requested_then_resolved,
            &snapshot
        ));
        assert!(event_batch_requires_snapshot(
            &completed_then_acknowledged,
            &snapshot
        ));
        assert!(!event_batch_requires_snapshot(
            &[transition(22, WorkState::Running)],
            &snapshot
        ));
    }

    #[test]
    fn notch_projection_is_a_strict_privacy_allowlist() {
        let snapshot = WorkSnapshot {
            schema_version: 1,
            cursor: EventCursor::new(7),
            daemon_generation: DaemonGeneration::new("daemon-fixture"),
            works: vec![WorkRecord {
                identity: bagentd::work_coordinator::WorkIdentity::new("opaque-work"),
                origin: WorkOrigin::Conversation {
                    current_chat_identity: CurrentChatIdentity::new("opaque-chat"),
                    conversation_turn_identity: ConversationTurnIdentity::new("opaque-turn"),
                },
                state: WorkState::Running,
                revision: WorkRevision::new(2),
                activity: None,
            }],
            automation_runs: vec![],
            approvals: vec![],
            interruptions: vec![],
            model_runtime_generation: None,
            model_runtime_trusted: true,
        };
        let context = NotchProjectionContext {
            model_phase: "ready",
            automation_names: HashMap::new(),
            automation_definition_identities: HashMap::new(),
            automation_session_identities: HashMap::new(),
            terminal_attention: HashMap::new(),
            terminal_orders: HashMap::new(),
            queue_positions: HashMap::new(),
            claimed_orders: HashMap::from([("opaque-work".to_owned(), 1)]),
        };

        let value = notch_snapshot_value(&snapshot, &context);
        let work = value["works"][0].as_object().unwrap();
        let actual = work
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let expected = [
            "identity",
            "revision",
            "origin",
            "state",
            "activity",
            "queuePosition",
            "automationDisplayName",
            "automationDefinitionIdentity",
            "automationSessionIdentity",
            "terminalAttention",
            "terminalOrder",
            "claimedOrder",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();

        assert_eq!(actual, expected);
        assert_eq!(value["model"], "ready");
        let serialized = value.to_string();
        for forbidden in [
            "prompt",
            "reasoning",
            "toolArguments",
            "connectorIdentifier",
            "evidenceContent",
            "providerError",
            "credential",
            "modelOutput",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }

    #[test]
    fn notch_projection_emits_revisioned_terminal_attention_and_destination() {
        let snapshot = WorkSnapshot {
            schema_version: 1,
            cursor: EventCursor::new(12),
            daemon_generation: DaemonGeneration::new("daemon-terminal"),
            works: vec![WorkRecord {
                identity: WorkIdentity::new("work-terminal"),
                origin: WorkOrigin::Automation {
                    automation_run_identity: bagentd::work_coordinator::AutomationRunIdentity::new(
                        "run-terminal",
                    ),
                    automation_session_identity:
                        bagentd::work_coordinator::AutomationSessionIdentity::new(
                            "session-terminal",
                        ),
                    historical_automation_identity:
                        bagentd::work_coordinator::AutomationDefinitionIdentity::new(
                            "definition-terminal",
                        ),
                    frozen_definition_revision:
                        bagentd::work_coordinator::AutomationDefinitionRevision::new(3),
                },
                state: WorkState::Failed,
                revision: WorkRevision::new(4),
                activity: None,
            }],
            automation_runs: vec![],
            approvals: vec![],
            interruptions: vec![],
            model_runtime_generation: None,
            model_runtime_trusted: true,
        };
        let context = NotchProjectionContext {
            model_phase: "ready",
            automation_names: HashMap::from([(
                "work-terminal".to_owned(),
                "Saved name".to_owned(),
            )]),
            automation_definition_identities: HashMap::from([(
                "work-terminal".to_owned(),
                "definition-terminal".to_owned(),
            )]),
            automation_session_identities: HashMap::from([(
                "work-terminal".to_owned(),
                "session-terminal".to_owned(),
            )]),
            terminal_attention: HashMap::from([("work-terminal".to_owned(), "failed")]),
            terminal_orders: HashMap::from([("work-terminal".to_owned(), 12)]),
            queue_positions: HashMap::new(),
            claimed_orders: HashMap::from([("work-terminal".to_owned(), 1)]),
        };

        let value = notch_snapshot_value(&snapshot, &context);
        assert_eq!(value["works"][0]["terminalAttention"], "failed");
        assert_eq!(value["works"][0]["terminalOrder"], 12);
        assert_eq!(
            value["works"][0]["automationDefinitionIdentity"],
            "definition-terminal"
        );
        assert_eq!(
            value["works"][0]["automationSessionIdentity"],
            "session-terminal"
        );
    }

    #[test]
    fn tavily_configuration_status_never_exposes_credential_material() {
        let configuration = TavilyConfiguration::pending();
        assert_eq!(configuration.status(), TavilyConfigurationStatus::Pending);

        configuration.apply(None).unwrap();
        assert_eq!(configuration.status(), TavilyConfigurationStatus::Absent);
        assert!(configuration.credential().is_none());

        let credential = String::from_iter(std::iter::repeat_n('k', 32));
        configuration.apply(Some(credential)).unwrap();
        assert_eq!(
            configuration.status(),
            TavilyConfigurationStatus::Configured
        );
        assert!(configuration.credential().is_some());

        let serialized = serde_json::to_string(&configuration.status()).unwrap();
        assert_eq!(serialized, "\"configured\"");
    }

    #[cfg(not(feature = "stage8-acceptance"))]
    #[tokio::test]
    async fn ordinary_build_exposes_no_acceptance_fixture_route() {
        assert_eq!(
            stage8_acceptance_not_found_handler().await,
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn attachment_upload_policy_rejects_images_for_text_only_basert() {
        assert!(!attachment_mime_is_supported("image/png"));
        assert!(!attachment_mime_is_supported("image/jpeg"));
        assert!(attachment_mime_is_supported("application/pdf"));
        assert!(attachment_mime_is_supported("text/plain"));
    }

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
    match mail.hydrate_message(rowid).await {
        Ok(Some(hydrated)) => {
            let msg = hydrated.message;
            let unavailable = match hydrated.state {
                apple_mail_connector::MailBodyHydrationState::Unavailable => Some(
                    "[body unavailable locally — call mail_open with this rowid to open it in Mail.app]",
                ),
                apple_mail_connector::MailBodyHydrationState::AutomationDenied => {
                    Some("[body unavailable — Mail.app Automation access was denied]")
                }
                apple_mail_connector::MailBodyHydrationState::AutomationTimedOut => {
                    Some("[body unavailable — Mail.app Automation timed out]")
                }
                apple_mail_connector::MailBodyHydrationState::AutomationFailed => {
                    Some("[body unavailable — Mail.app Automation failed]")
                }
                apple_mail_connector::MailBodyHydrationState::Readable
                | apple_mail_connector::MailBodyHydrationState::Empty => None,
            };
            let body: String = unavailable
                .or(msg.body.as_deref())
                .unwrap_or("")
                .chars()
                .take(4_000)
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
        Ok(None) => ("No message with that rowid.".to_string(), None),
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

// ── Web tools ────────────────────────────────────────────────────────────────

async fn tool_web_search(args: &serde_json::Value) -> String {
    let Some(query) = json_str_arg(args, "query") else {
        return "query is required.".to_string();
    };
    let lang = match json_str_arg(args, "lang").as_deref() {
        Some("sk") => "sk",
        _ => "en",
    };
    let result = evidence::production_web_search(&query, lang).await;
    evidence::render_legacy_search(&result, &query)
}

async fn tool_web_fetch(args: &serde_json::Value) -> String {
    let Some(url) = json_str_arg(args, "url") else {
        return "url is required.".to_string();
    };
    let result = evidence::production_web_fetch(&url).await;
    evidence::render_legacy_fetch(&result)
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
