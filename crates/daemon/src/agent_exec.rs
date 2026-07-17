//! Reusable agent execution service: one tool-calling loop, tool registry,
//! policy gate, and approval boundary shared by foreground chat and unattended
//! automations. The HTTP chat route and the scheduler both call into here —
//! neither duplicates the loop.
//!
//! Safety model:
//! - The model is never trusted for authorization; every tool call passes
//!   through `Gate` (rules engine with the actual serialized arguments) and,
//!   for side-effecting tools in unattended runs, a forced fresh approval.
//! - Unknown or unclassified tools fail closed in unattended runs.
//! - Approval descriptions identify the originating automation.

use futures_util::StreamExt;
use ollama_connector::{ChatStreamEvent, Message, ToolCall as OllamaToolCall, ToolDef};
use serde_json::json;
use tokio::sync::mpsc;

use bagent_rules::{ApprovalLevel, RuleEngine};
use filesystem_connector::{open as fs_open, search as fs_search, FileSearchRequest, OpenResponse, ReadTextRequest};
use whatsapp_connector::WhatsappSendTarget;

use crate::{
    audit_fs, json_str_arg, request_tool_approval, run_aerospace, save_last_file_ref,
    save_last_mail_ref, save_last_odoo_ref, save_last_whatsapp_ref, sha256_str, tool_mail_list_inbox,
    tool_mail_open, tool_mail_read, tool_mail_search, tool_notes_read, tool_notes_search, tool_odoo,
    tool_web_fetch, tool_web_search, tool_whatsapp_chat_messages, tool_whatsapp_list_chats, AppState,
    FileRef,
};

/// Where an execution came from. Trusted metadata — set by the daemon, never
/// by model output or stored prompts.
#[derive(Debug, Clone)]
pub(crate) enum ExecOrigin {
    /// Interactive chat with the user watching the stream.
    Chat,
    /// Unattended scheduled/run-now automation.
    #[allow(dead_code)] // constructed by the automations scheduler
    Automation {
        automation_name: String,
    },
}

impl ExecOrigin {
    pub(crate) fn unattended(&self) -> bool {
        matches!(self, ExecOrigin::Automation { .. })
    }

    /// Approval descriptions must identify the originating automation.
    fn describe(&self, description: &str) -> String {
        match self {
            ExecOrigin::Chat => description.to_string(),
            ExecOrigin::Automation { automation_name } => {
                format!("Automatizácia „{automation_name}“: {description}")
            }
        }
    }
}

/// Pluggable event sink. Chat forwards to the SSE stream; automations forward
/// to the daemon event broadcast (or discard).
#[derive(Clone)]
pub(crate) struct EventSink(mpsc::Sender<serde_json::Value>);

impl EventSink {
    pub(crate) fn new(tx: mpsc::Sender<serde_json::Value>) -> Self {
        Self(tx)
    }

    /// Returns false when the receiver is gone (client disconnected).
    pub(crate) async fn emit(&self, v: serde_json::Value) -> bool {
        self.0.send(v).await.is_ok()
    }
}

#[derive(Debug)]
#[allow(dead_code)] // run metadata consumed by the automations scheduler
pub(crate) struct ExecOutcome {
    pub final_text: String,
    pub tool_calls_used: usize,
    /// Gated actions the user denied (or that timed out) during this run.
    pub approvals_denied: usize,
}

#[derive(Debug)]
#[allow(dead_code)] // error detail consumed by the automations scheduler
pub(crate) enum ExecError {
    /// The event receiver went away (chat client disconnected).
    SinkClosed,
    /// The model stream failed.
    Model(String),
}

/// Whether a tool only reads local/remote data or has side effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolKind {
    ReadOnly,
    SideEffect,
}

/// Explicit classification of every registered tool. `None` means unknown —
/// unattended runs fail closed on it.
pub(crate) fn classify_tool(name: &str) -> Option<ToolKind> {
    match name {
        "mail_search" | "mail_list_inbox" | "mail_read" | "notes_search" | "notes_read"
        | "filesystem_search_files" | "filesystem_read_text" | "whatsapp_list_chats"
        | "whatsapp_chat_messages" | "odoo_search_partners" | "odoo_my_invoices"
        | "odoo_my_helpdesk_tickets" | "odoo_get_record" | "web_search" | "web_fetch" => {
            Some(ToolKind::ReadOnly)
        }
        "mail_open" | "filesystem_open_file" | "filesystem_open_file_with"
        | "filesystem_reveal_in_finder" | "filesystem_open_folder" | "macos_open_app"
        | "macos_focus_app" | "macos_switch_workspace" | "whatsapp_send_message" => {
            Some(ToolKind::SideEffect)
        }
        _ => None,
    }
}

/// Policy gate: rules-engine verdict on the actual serialized arguments, with
/// unattended escalation — side-effecting tools never run on `auto` without a
/// fresh approval when nobody is watching.
pub(crate) struct Gate<'a> {
    rules: &'a RuleEngine,
    unattended: bool,
}

impl<'a> Gate<'a> {
    pub(crate) fn new(rules: &'a RuleEngine, origin: &ExecOrigin) -> Self {
        Self { rules, unattended: origin.unattended() }
    }

    pub(crate) fn level(
        &self,
        rule: &str,
        args: &serde_json::Value,
        kind: ToolKind,
    ) -> ApprovalLevel {
        let verdict = self.rules.check(rule, &args.to_string());
        match (self.unattended, kind, verdict) {
            (true, ToolKind::SideEffect, ApprovalLevel::Auto) => ApprovalLevel::Ask,
            (_, _, v) => v,
        }
    }
}

#[cfg(test)]
pub(crate) fn escalate_for_test(
    unattended: bool,
    kind: ToolKind,
    verdict: ApprovalLevel,
) -> ApprovalLevel {
    match (unattended, kind, verdict) {
        (true, ToolKind::SideEffect, ApprovalLevel::Auto) => ApprovalLevel::Ask,
        (_, _, v) => v,
    }
}

/// Build the per-turn tool registry from available connectors. `vision` turns
/// get no tools — the vision model answers directly from injected context.
pub(crate) async fn build_tools(state: &AppState, vision: bool) -> Vec<ToolDef> {
    let mut tools: Vec<ToolDef> = Vec::new();
    if vision {
        return tools;
    }
    if state.mail.is_some() {
        tools.push(ToolDef::function(
            "mail_search",
            "Search the user's Apple Mail. Returns message headers (rowid, subject, sender, date). \
             Use mail_read with a rowid to get a message body. \
             When the user asks what a mail says or about its content, immediately follow up \
             with mail_read on the best-matching rowid — reading is safe, never ask permission first. \
             Put the sender's email address or name in `sender` when the user asks about mail from someone. \
             IMPORTANT: Never describe a message that this tool did not return.",
            json!({
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
        tools.push(ToolDef::function(
            "mail_list_inbox",
            "List the most recent inbox messages, optionally unread only.",
            json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "Max results, default 10."},
                    "unread_only": {"type": "boolean"}
                }
            }),
        ));
        tools.push(ToolDef::function(
            "mail_read",
            "Read the full body of a mail message by rowid (from mail_search / mail_list_inbox). \
             Call this without asking whenever the user wants a message's content, summary, or details.",
            json!({
                "type": "object",
                "properties": {"rowid": {"type": "integer"}},
                "required": ["rowid"]
            }),
        ));
        tools.push(ToolDef::function(
            "mail_open",
            "Open a mail message in the Mail app by rowid.",
            json!({
                "type": "object",
                "properties": {"rowid": {"type": "integer"}},
                "required": ["rowid"]
            }),
        ));
    }
    if state.fs.is_some() {
        tools.push(ToolDef::function(
            "filesystem_search_files",
            "Search the user's Mac for files by name or content using macOS Spotlight. \
             Use multiple Slovak/English synonym terms for best recall on Slovak documents. \
             IMPORTANT: Never name or describe a file that was not returned by this tool.",
            json!({
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
        tools.push(ToolDef::function(
            "filesystem_read_text",
            "Read the text content of a local file (PDF, Word, Excel, or plain text). \
             Use this to inspect candidate files returned by filesystem_search_files.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Absolute path to the file."}
                },
                "required": ["path"]
            }),
        ));
        tools.push(ToolDef::function(
            "filesystem_open_file",
            "Open a local file in its default application. Requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Absolute path to the file."}
                },
                "required": ["path"]
            }),
        ));
        tools.push(ToolDef::function(
            "filesystem_open_file_with",
            "Open a local file in a specific application. Requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "app": {"type": "string", "description": "App name, e.g. 'Microsoft Excel', 'Preview'."}
                },
                "required": ["path", "app"]
            }),
        ));
        tools.push(ToolDef::function(
            "filesystem_reveal_in_finder",
            "Reveal a local file in the macOS Finder.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                },
                "required": ["path"]
            }),
        ));
        tools.push(ToolDef::function(
            "macos_open_app",
            "Launch or focus a macOS application by name.",
            json!({
                "type": "object",
                "properties": {
                    "app": {"type": "string", "description": "App name, e.g. 'Mail', 'Finder', 'Preview'."}
                },
                "required": ["app"]
            }),
        ));
    }
    if state.notes.is_some() {
        tools.push(ToolDef::function(
            "notes_search",
            "Search Apple Notes by title/snippet. Returns note metadata; use notes_read for the body.",
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "description": "Max results, default 10."}
                },
                "required": ["query"]
            }),
        ));
        tools.push(ToolDef::function(
            "notes_read",
            "Read the body of an Apple Note by its coredata_id (from notes_search).",
            json!({
                "type": "object",
                "properties": {"coredata_id": {"type": "string"}},
                "required": ["coredata_id"]
            }),
        ));
    }
    tools.push(ToolDef::function(
        "web_search",
        "Search the public web (DuckDuckGo + Wikipedia). Returns result lines: title | url | snippet. \
         Use for facts, current events, prices, or to identify an entity (e.g. what company makes a product) \
         before searching mail or files. Follow up with web_fetch on the best URL when snippets are not enough. \
         IMPORTANT: Answer factual questions ONLY from these results, cite the source URL, \
         and say the answer was not found rather than guessing.",
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query, in the language most likely to have results (usually English)."},
                "lang": {"type": "string", "description": "Wikipedia language code, 'en' or 'sk'. Default 'en'."}
            },
            "required": ["query"]
        }),
    ));
    tools.push(ToolDef::function(
        "web_fetch",
        "Download a web page by URL and return its readable text (capped), \
         plus a Links section of same-site links. Use on a URL from web_search \
         or one the user provided. When the answer lives on a subpage (daily menu, \
         news article), call web_fetch again on the most promising listed link. \
         IMPORTANT: Never describe page content this tool did not return.",
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Full http(s) URL to fetch."}
            },
            "required": ["url"]
        }),
    ));
    tools.push(ToolDef::function(
        "macos_switch_workspace",
        "Switch to an AeroSpace window-manager workspace by name/number.",
        json!({
            "type": "object",
            "properties": {"workspace": {"type": "string"}},
            "required": ["workspace"]
        }),
    ));
    // WhatsApp connector always exists; calls report politely when the bridge is down.
    tools.push(ToolDef::function(
        "whatsapp_list_chats",
        "List the user's recent WhatsApp chats (chat_id, contact name, last message).",
        json!({
            "type": "object",
            "properties": {"limit": {"type": "integer", "description": "Max chats, default 20."}}
        }),
    ));
    tools.push(ToolDef::function(
        "whatsapp_chat_messages",
        "Read recent messages of one WhatsApp chat by chat_id (from whatsapp_list_chats).",
        json!({
            "type": "object",
            "properties": {
                "chat_id": {"type": "string"},
                "limit": {"type": "integer", "description": "Max messages, default 20."}
            },
            "required": ["chat_id"]
        }),
    ));
    tools.push(ToolDef::function(
        "whatsapp_send_message",
        "Send ONE WhatsApp text message to a chat. Always requires explicit user approval.",
        json!({
            "type": "object",
            "properties": {
                "chat_id": {"type": "string"},
                "message": {"type": "string"}
            },
            "required": ["chat_id", "message"]
        }),
    ));
    if state.odoo.read().await.is_some() {
        tools.push(ToolDef::function(
            "odoo_search_partners",
            "Search Odoo partners (customers/suppliers) by name or email.",
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "description": "Max results, default 10."}
                },
                "required": ["query"]
            }),
        ));
        tools.push(ToolDef::function(
            "odoo_my_invoices",
            "List Odoo invoices. open_only=true returns only unpaid/partially-paid ones.",
            json!({
                "type": "object",
                "properties": {
                    "open_only": {"type": "boolean"},
                    "limit": {"type": "integer", "description": "Max results, default 10."}
                }
            }),
        ));
        tools.push(ToolDef::function(
            "odoo_my_helpdesk_tickets",
            "List the user's Odoo helpdesk tickets. open_only=true excludes closed stages.",
            json!({
                "type": "object",
                "properties": {
                    "open_only": {"type": "boolean"},
                    "limit": {"type": "integer", "description": "Max results, default 10."}
                }
            }),
        ));
        tools.push(ToolDef::function(
            "odoo_get_record",
            "Read a single Odoo record by model and id, e.g. model='res.partner', id=42.",
            json!({
                "type": "object",
                "properties": {
                    "model": {"type": "string"},
                    "id": {"type": "integer"}
                },
                "required": ["model", "id"]
            }),
        ));
    }
    tools
}

/// Run the agent loop to completion. `messages` is the fully built prompt
/// (system layers + history + current user turn); `tools` from `build_tools`.
/// Existing budgets preserved: max 5 rounds, 8 tool calls per turn.
pub(crate) async fn run_agent_loop(
    state: &AppState,
    sink: &EventSink,
    origin: &ExecOrigin,
    session_id: &str,
    model: &str,
    mut messages: Vec<Message>,
    tools: Vec<ToolDef>,
) -> Result<ExecOutcome, ExecError> {
    let db = &state.db;
    let rules = state.rules.clone();
    let gate = Gate::new(&rules, origin);
    let mail = &state.mail;
    let notes = &state.notes;
    let fs_exec = &state.fs;
    let pending_approvals = &state.pending_approvals;
    let runtime_refs = &state.runtime_refs;
    let ollama = &state.ollama;

    let mut full_response = String::new();
    let mut approvals_denied: usize = 0;
    let mut tool_calls_used: usize = 0;

    if tools.is_empty() {
        // Vision turns / no connectors: single streamed answer, no tools.
        let token_stream = ollama.chat_stream(model.to_string(), messages.clone());
        tokio::pin!(token_stream);
        while let Some(result) = token_stream.next().await {
            match result {
                Ok(token) => {
                    full_response.push_str(&token);
                    if !sink.emit(json!({"type":"token","content":token})).await {
                        return Err(ExecError::SinkClosed);
                    }
                }
                Err(e) => {
                    let _ = sink
                        .emit(json!({"type":"error","message": e.to_string()}))
                        .await;
                    return Err(ExecError::Model(e.to_string()));
                }
            }
        }
        return Ok(ExecOutcome { final_text: full_response, tool_calls_used, approvals_denied });
    }

    let mut found_file_ref: Option<FileRef> = None;
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
        let stream = ollama.chat_stream_with_tools(model.to_string(), messages.clone(), round_tools);
        tokio::pin!(stream);

        let mut round_text = String::new();
        let mut round_calls: Vec<OllamaToolCall> = Vec::new();
        while let Some(ev) = stream.next().await {
            match ev {
                Ok(ChatStreamEvent::Delta(token)) => {
                    round_text.push_str(&token);
                    if !sink.emit(json!({"type":"token","content":token})).await {
                        return Err(ExecError::SinkClosed);
                    }
                }
                Ok(ChatStreamEvent::ToolCalls(calls)) => round_calls.extend(calls),
                Err(e) => {
                    let _ = sink
                        .emit(json!({"type":"error","message": e.to_string()}))
                        .await;
                    return Err(ExecError::Model(e.to_string()));
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
            let _ = sink.emit(json!({"type":"tool_call","tool": fn_name})).await;
            audit_fs(db, "tool_call", &json!({"tool": fn_name, "unattended": origin.unattended()}));

            let tool_kind = classify_tool(fn_name);

            let tool_result: String = if tool_calls_used > MAX_TOOL_CALLS {
                "Tool budget exhausted — answer now using what you have.".to_string()
            } else if origin.unattended() && tool_kind.is_none() {
                // Fail closed: unattended runs never execute unmapped operations.
                let _ = sink
                    .emit(json!({"type":"tool_blocked","tool": fn_name}))
                    .await;
                format!("Unknown tool: {fn_name}. Not permitted in unattended runs — answer with what you have.")
            } else {
                match fn_name.as_str() {
                    // ── Mail ──────────────────────────────────────────
                    tool @ ("mail_search" | "mail_list_inbox" | "mail_read" | "mail_open") => {
                        let kind = tool_kind.unwrap_or(ToolKind::SideEffect);
                        match (mail, gate.level("mail_inbox", args, kind)) {
                            (None, _) => {
                                "Apple Mail connector unavailable (Full Disk Access not granted)."
                                    .to_string()
                            }
                            (_, ApprovalLevel::Forbidden) => {
                                let _ = sink
                                    .emit(json!({"type":"tool_blocked","tool":"mail_inbox"}))
                                    .await;
                                "Mail access blocked by rules.".to_string()
                            }
                            (Some(m), level) => {
                                let approved = match level {
                                    ApprovalLevel::Ask => {
                                        request_tool_approval(
                                            db,
                                            pending_approvals,
                                            sink,
                                            "mail_inbox",
                                            &origin.describe("Čítanie poštovej schránky (Apple Mail)"),
                                        )
                                        .await
                                    }
                                    _ => true,
                                };
                                if !approved {
                                    approvals_denied += 1;
                                    "Mail access not approved by the user.".to_string()
                                } else {
                                    match tool {
                                        "mail_search" => {
                                            let (result, mail_ref) = tool_mail_search(m, args).await;
                                            if let Some(ref r) = mail_ref {
                                                let _ = sink
                                                    .emit(json!({
                                                        "type": "mail_found",
                                                        "rowid": r.rowid,
                                                        "message_id": r.message_id,
                                                        "subject": r.subject,
                                                        "sender": r.sender,
                                                        "auto_open": false,
                                                    }))
                                                    .await;
                                                save_last_mail_ref(runtime_refs, session_id, r).await;
                                            }
                                            result
                                        }
                                        "mail_list_inbox" => tool_mail_list_inbox(m, args).await,
                                        "mail_read" => {
                                            let (result, mail_ref) = tool_mail_read(m, args).await;
                                            if let Some(ref r) = mail_ref {
                                                save_last_mail_ref(runtime_refs, session_id, r).await;
                                            }
                                            result
                                        }
                                        _ => tool_mail_open(m, args).await,
                                    }
                                }
                            }
                        }
                    }

                    // ── Notes ─────────────────────────────────────────
                    tool @ ("notes_search" | "notes_read") => match notes {
                        None => "Apple Notes connector unavailable.".to_string(),
                        Some(n) => match gate.level("notes_search", args, ToolKind::ReadOnly) {
                            ApprovalLevel::Forbidden => "Notes access blocked by rules.".to_string(),
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
                    "whatsapp_list_chats" => tool_whatsapp_list_chats(&state.whatsapp, args).await,
                    "whatsapp_chat_messages" => {
                        let (result, wa_ref) =
                            tool_whatsapp_chat_messages(&state.whatsapp, args).await;
                        if let Some(ref r) = wa_ref {
                            save_last_whatsapp_ref(runtime_refs, session_id, r).await;
                        }
                        result
                    }
                    "whatsapp_send_message" => {
                        let chat_id = args["chat_id"].as_str().unwrap_or_default().to_string();
                        let text = args["message"].as_str().unwrap_or_default().to_string();
                        if chat_id.is_empty() || text.is_empty() {
                            "chat_id and message are required.".to_string()
                        } else {
                            let approved = request_tool_approval(
                                db,
                                pending_approvals,
                                sink,
                                "whatsapp.send_message",
                                &origin.describe(&format!("WhatsApp → {chat_id}: {text}")),
                            )
                            .await;
                            if !approved {
                                approvals_denied += 1;
                                "User did not approve sending the message.".to_string()
                            } else {
                                match state
                                    .whatsapp
                                    .send_message(WhatsappSendTarget::ChatId(chat_id), &text)
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
                            None => "Odoo not connected — connect it in Settings first.".to_string(),
                            Some(o) => {
                                let (result, odoo_ref) = tool_odoo(o, tool, args).await;
                                if let Some(ref r) = odoo_ref {
                                    let _ = sink
                                        .emit(json!({
                                            "type": "odoo_found",
                                            "model": r.model,
                                            "record_id": r.id,
                                            "name": r.name,
                                            "url": r.url,
                                        }))
                                        .await;
                                    save_last_odoo_ref(runtime_refs, session_id, r).await;
                                }
                                result
                            }
                        }
                    }

                    // ── Window management (AeroSpace) ─────────────────
                    "macos_switch_workspace" => {
                        let level = gate.level("macos.switch_workspace", args, ToolKind::SideEffect);
                        let approved = match level {
                            ApprovalLevel::Forbidden => {
                                let _ = sink
                                    .emit(json!({"type":"tool_blocked","tool":"macos.switch_workspace"}))
                                    .await;
                                false
                            }
                            ApprovalLevel::Ask => {
                                let ok = request_tool_approval(
                                    db,
                                    pending_approvals,
                                    sink,
                                    "macos.switch_workspace",
                                    &origin.describe(&format!(
                                        "Prepnúť workspace: {}",
                                        args["workspace"].as_str().unwrap_or("?")
                                    )),
                                )
                                .await;
                                if !ok {
                                    approvals_denied += 1;
                                }
                                ok
                            }
                            ApprovalLevel::Auto => true,
                        };
                        if !approved {
                            "Workspace switch not approved.".to_string()
                        } else {
                            match json_str_arg(args, "workspace") {
                                None => "workspace is required.".to_string(),
                                Some(ws) => match run_aerospace(&["workspace", &ws]).await {
                                    Ok(_) => format!("Switched to workspace {ws}."),
                                    Err(e) => format!("AeroSpace error: {e}"),
                                },
                            }
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
                            let roots: Option<Vec<String>> = args["roots"].as_array().map(|a| {
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
                            let search_contents = args["search_contents"].as_bool().unwrap_or(false);
                            let max_results =
                                args["max_results"].as_u64().map(|n| n as usize).unwrap_or(10);

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
                                        db,
                                        "filesystem_search",
                                        &json!({
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
                                                kind: format!("{:?}", top.kind).to_lowercase(),
                                            });
                                        }
                                    }
                                    serde_json::to_string(&resp).unwrap_or_else(|_| "[]".to_string())
                                }
                                Ok(Err(e)) => format!("{{\"error\":\"{}\"}}", e),
                                Err(e) => format!("{{\"error\":\"{}\"}}", e),
                            }
                        }
                    },

                    "filesystem_read_text" => match fs_exec.as_ref() {
                        None => "Filesystem connector unavailable.".to_string(),
                        Some(fs_c) => {
                            let path = args["path"].as_str().unwrap_or("").to_string();
                            let req = ReadTextRequest { path, max_bytes: None, around_line: None };
                            let policy = fs_c.policy.clone();
                            match tokio::task::spawn_blocking(move || {
                                fs_search::read_text_sync(&policy, req)
                            })
                            .await
                            {
                                Ok(Ok(resp)) => {
                                    // Cap content to avoid huge context
                                    let content: String = resp.content.chars().take(4000).collect();
                                    let truncated_note =
                                        if resp.truncated { " [truncated]" } else { "" };
                                    format!("[File: {}]\n{}{}", resp.path, content, truncated_note)
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
                        let approval_level = gate.level(rule_name, args, ToolKind::SideEffect);
                        let approved = match approval_level {
                            ApprovalLevel::Auto => true,
                            ApprovalLevel::Ask => {
                                let ok = request_tool_approval(
                                    db,
                                    pending_approvals,
                                    sink,
                                    rule_name,
                                    &origin.describe(&format!(
                                        "Open: {}",
                                        path.as_deref().or(app.as_deref()).unwrap_or("?")
                                    )),
                                )
                                .await;
                                if !ok {
                                    approvals_denied += 1;
                                }
                                ok
                            }
                            ApprovalLevel::Forbidden => {
                                let _ = sink
                                    .emit(json!({
                                        "type": "tool_blocked",
                                        "tool": rule_name
                                    }))
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
                                            db,
                                            &rule_name.replace('.', "_"),
                                            &json!({"app": a, "ok": true}),
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
                                    let result: anyhow::Result<OpenResponse> = match rule_name {
                                        "filesystem.open_file" => {
                                            if let Some(ref p) = path {
                                                fs_open::open_file(&fs_c.policy, p).await
                                            } else {
                                                Err(anyhow::anyhow!("no path"))
                                            }
                                        }
                                        "filesystem.open_file_with" => {
                                            if let (Some(ref p), Some(ref a)) = (&path, &app) {
                                                fs_open::open_file_with(&fs_c.policy, p, a).await
                                            } else {
                                                Err(anyhow::anyhow!("no path or app"))
                                            }
                                        }
                                        "filesystem.reveal_in_finder" => {
                                            if let Some(ref p) = path {
                                                fs_open::reveal_in_finder(&fs_c.policy, p).await
                                            } else {
                                                Err(anyhow::anyhow!("no path"))
                                            }
                                        }
                                        "filesystem.open_folder" => {
                                            if let Some(ref p) = path {
                                                fs_open::open_folder(&fs_c.policy, p).await
                                            } else {
                                                Err(anyhow::anyhow!("no path"))
                                            }
                                        }
                                        _ => Err(anyhow::anyhow!("unknown")),
                                    };
                                    match result {
                                        Ok(ref resp) => {
                                            let path_hash = path.as_deref().map(sha256_str);
                                            audit_fs(
                                                db,
                                                &rule_name.replace('.', "_"),
                                                &json!({
                                                    "path_hash": path_hash,
                                                    "app": app,
                                                    "ok": true
                                                }),
                                            );
                                            let _ = sink
                                                .emit(json!({
                                                    "type": "file_opened",
                                                    "path": resp.path,
                                                    "app": resp.app,
                                                    "action": resp.action,
                                                }))
                                                .await;
                                            format!(
                                                "Opened: {}",
                                                path.as_deref().or(app.as_deref()).unwrap_or("ok")
                                            )
                                        }
                                        Err(ref e) => {
                                            audit_fs(
                                                db,
                                                &rule_name.replace('.', "_"),
                                                &json!({
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

                    // ── Web (read-only; queries leave the device) ─────
                    tool @ ("web_search" | "web_fetch") => {
                        let rule_name = if tool == "web_search" { "web.search" } else { "web.fetch" };
                        match gate.level(rule_name, args, ToolKind::ReadOnly) {
                            ApprovalLevel::Forbidden => {
                                let _ = sink
                                    .emit(json!({"type":"tool_blocked","tool": rule_name}))
                                    .await;
                                "Web access blocked by rules.".to_string()
                            }
                            level => {
                                let approved = match level {
                                    ApprovalLevel::Ask => {
                                        let ok = request_tool_approval(
                                            db,
                                            pending_approvals,
                                            sink,
                                            rule_name,
                                            &origin.describe(&format!(
                                                "Web: {}",
                                                args["query"]
                                                    .as_str()
                                                    .or(args["url"].as_str())
                                                    .unwrap_or("?")
                                            )),
                                        )
                                        .await;
                                        if !ok {
                                            approvals_denied += 1;
                                        }
                                        ok
                                    }
                                    _ => true,
                                };
                                if !approved {
                                    "Web access not approved by the user.".to_string()
                                } else {
                                    let result = if tool == "web_search" {
                                        tool_web_search(args).await
                                    } else {
                                        tool_web_fetch(args).await
                                    };
                                    audit_fs(db, &rule_name.replace('.', "_"), &json!({"ok": true}));
                                    result
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
        save_last_file_ref(runtime_refs, session_id, fref).await;
    }

    Ok(ExecOutcome { final_text: full_response, tool_calls_used, approvals_denied })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_tool_is_classified() {
        // The registry names — build_tools needs a full AppState, so the list
        // is mirrored here; classify_tool is the contract under test.
        for name in [
            "mail_search", "mail_list_inbox", "mail_read", "mail_open",
            "filesystem_search_files", "filesystem_read_text", "filesystem_open_file",
            "filesystem_open_file_with", "filesystem_reveal_in_finder", "macos_open_app",
            "notes_search", "notes_read", "web_search", "web_fetch",
            "macos_switch_workspace", "whatsapp_list_chats", "whatsapp_chat_messages",
            "whatsapp_send_message", "odoo_search_partners", "odoo_my_invoices",
            "odoo_my_helpdesk_tickets", "odoo_get_record",
        ] {
            assert!(classify_tool(name).is_some(), "unclassified tool: {name}");
        }
    }

    #[test]
    fn unknown_tools_are_unclassified() {
        assert!(classify_tool("shell_exec").is_none());
        assert!(classify_tool("mail_send").is_none());
        assert!(classify_tool("").is_none());
    }

    #[test]
    fn side_effects_are_side_effects() {
        for name in [
            "mail_open", "filesystem_open_file", "filesystem_open_file_with",
            "filesystem_reveal_in_finder", "macos_open_app", "macos_focus_app",
            "macos_switch_workspace", "whatsapp_send_message",
        ] {
            assert_eq!(classify_tool(name), Some(ToolKind::SideEffect), "{name}");
        }
        for name in ["mail_search", "web_fetch", "odoo_get_record", "filesystem_read_text"] {
            assert_eq!(classify_tool(name), Some(ToolKind::ReadOnly), "{name}");
        }
    }

    #[test]
    fn unattended_escalates_auto_side_effects_to_ask() {
        use ApprovalLevel::*;
        // Unattended + side effect + auto → ask (fresh approval required).
        assert!(matches!(escalate_for_test(true, ToolKind::SideEffect, Auto), Ask));
        // Forbidden always stays forbidden.
        assert!(matches!(escalate_for_test(true, ToolKind::SideEffect, Forbidden), Forbidden));
        // Reads keep their rules verdict unattended.
        assert!(matches!(escalate_for_test(true, ToolKind::ReadOnly, Auto), Auto));
        // Attended behavior unchanged.
        assert!(matches!(escalate_for_test(false, ToolKind::SideEffect, Auto), Auto));
        assert!(matches!(escalate_for_test(false, ToolKind::ReadOnly, Ask), Ask));
    }
}
