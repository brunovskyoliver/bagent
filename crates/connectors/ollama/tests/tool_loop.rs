//! Live tool-calling regression tests (require Ollama with qwen3:8b).
//!
//! Run with: cargo test -p ollama-connector -- --include-ignored

use futures_util::StreamExt;
use ollama_connector::{ChatStreamEvent, Message, OllamaClient, ToolDef};

const MODEL: &str = "qwen3:8b";

fn mail_search_tool() -> ToolDef {
    ToolDef::function(
        "mail_search",
        "Search the user's Apple Mail. Returns message headers. \
         Put the sender's email address or name in `sender` when the user asks about mail from someone.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "sender": {"type": "string", "description": "Sender email address or name."},
                "subject": {"type": "string"},
                "limit": {"type": "integer"}
            }
        }),
    )
}

/// The regression that motivated the tool loop: a query naming a sender email
/// address must produce a mail_search tool call carrying that address.
#[tokio::test]
#[ignore]
async fn mail_query_with_email_address_triggers_mail_search() {
    let client = OllamaClient::new("http://127.0.0.1:11434");
    assert!(client.is_up().await, "Ollama not reachable");

    let messages = vec![
        Message::system("You are a local assistant. Use tools to access the user's data; never invent mail content."),
        Message::user("show me the most recent emails from tomas.juricek@novem.sk"),
    ];
    let stream = client.chat_stream_with_tools(MODEL.into(), messages, vec![mail_search_tool()]);
    tokio::pin!(stream);

    let mut calls = Vec::new();
    while let Some(ev) = stream.next().await {
        if let ChatStreamEvent::ToolCalls(c) = ev.expect("stream error") {
            calls.extend(c);
        }
    }

    assert!(!calls.is_empty(), "model did not call any tool");
    let call = &calls[0];
    assert_eq!(call.function.name, "mail_search");
    let args = serde_json::to_string(&call.function.arguments).unwrap();
    assert!(
        args.contains("tomas.juricek") || args.contains("novem.sk"),
        "sender not extracted into tool args: {args}"
    );
}

/// Slovak phrasing must also produce a mail tool call.
#[tokio::test]
#[ignore]
async fn slovak_mail_query_triggers_mail_search() {
    let client = OllamaClient::new("http://127.0.0.1:11434");
    assert!(client.is_up().await, "Ollama not reachable");

    let messages = vec![
        Message::system("You are a local assistant. Use tools to access the user's data; never invent mail content."),
        Message::user("nájdi mi posledné maily od tomas.juricek@novem.sk"),
    ];
    let stream = client.chat_stream_with_tools(MODEL.into(), messages, vec![mail_search_tool()]);
    tokio::pin!(stream);

    let mut got_call = false;
    while let Some(ev) = stream.next().await {
        if let ChatStreamEvent::ToolCalls(c) = ev.expect("stream error") {
            got_call = got_call || c.iter().any(|t| t.function.name == "mail_search");
        }
    }
    assert!(got_call, "model did not call mail_search on Slovak query");
}

/// After a tool result is fed back, the model must answer (no more tool calls
/// needed) and ground the answer in the returned data.
#[tokio::test]
#[ignore]
async fn tool_result_round_trip_produces_grounded_answer() {
    let client = OllamaClient::new("http://127.0.0.1:11434");
    assert!(client.is_up().await, "Ollama not reachable");

    let mut messages = vec![
        Message::system("You are a local assistant. Use tools to access the user's data; never invent mail content."),
        Message::user("show me the most recent email from tomas.juricek@novem.sk"),
    ];
    // Simulate the loop: round 1 → tool call
    let stream = client.chat_stream_with_tools(MODEL.into(), messages.clone(), vec![mail_search_tool()]);
    tokio::pin!(stream);
    let mut calls = Vec::new();
    while let Some(ev) = stream.next().await {
        if let ChatStreamEvent::ToolCalls(c) = ev.expect("stream error") {
            calls.extend(c);
        }
    }
    assert!(!calls.is_empty());
    messages.push(Message::assistant_tool_calls(calls.clone()));
    messages.push(Message::tool_result(
        "mail_search",
        r#"[{"rowid":42,"subject":"Cenová ponuka Novem","sender":"tomas.juricek@novem.sk","date":"2026-07-01T09:00:00+00:00","is_read":false}]"#,
    ));

    // Round 2 → answer, no tools passed
    let stream2 = client.chat_stream_with_tools(MODEL.into(), messages, vec![]);
    tokio::pin!(stream2);
    let mut answer = String::new();
    while let Some(ev) = stream2.next().await {
        if let ChatStreamEvent::Delta(t) = ev.expect("stream error") {
            answer.push_str(&t);
        }
    }
    assert!(
        answer.contains("Cenová ponuka") || answer.contains("Novem") || answer.contains("novem.sk"),
        "answer not grounded in tool result: {answer}"
    );
}

fn web_search_tool() -> ToolDef {
    ToolDef::function(
        "web_search",
        "Search the public web (DuckDuckGo + Wikipedia). Returns result lines: title | url | snippet. \
         Use for facts, current events, prices, or to identify an entity. \
         IMPORTANT: Answer factual questions ONLY from these results, cite the source URL, \
         and say the answer was not found rather than guessing.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "lang": {"type": "string"}
            },
            "required": ["query"]
        }),
    )
}

/// A factual web question must produce a web_search call, and the final answer
/// must be grounded in the injected results (no invented sources).
#[tokio::test]
#[ignore]
async fn web_question_triggers_web_search_and_grounded_answer() {
    let client = OllamaClient::new("http://127.0.0.1:11434");
    assert!(client.is_up().await, "Ollama not reachable");

    let mut messages = vec![
        Message::system("You are a local assistant. Use tools for facts; never guess."),
        Message::user("Which company develops the tool called Claude Code? Search the web."),
    ];
    let stream = client.chat_stream_with_tools(MODEL.into(), messages.clone(), vec![web_search_tool()]);
    tokio::pin!(stream);
    let mut calls = Vec::new();
    while let Some(ev) = stream.next().await {
        if let ChatStreamEvent::ToolCalls(c) = ev.expect("stream error") {
            calls.extend(c);
        }
    }
    assert!(!calls.is_empty(), "model did not call any tool");
    assert_eq!(calls[0].function.name, "web_search");

    messages.push(Message::assistant_tool_calls(calls.clone()));
    messages.push(Message::tool_result(
        "web_search",
        "Web results (title | url | snippet):\n\
         Claude Code | https://www.anthropic.com/product/claude-code | Claude Code is an agentic coding tool by Anthropic.",
    ));
    let stream2 = client.chat_stream_with_tools(MODEL.into(), messages, vec![]);
    tokio::pin!(stream2);
    let mut answer = String::new();
    while let Some(ev) = stream2.next().await {
        if let ChatStreamEvent::Delta(t) = ev.expect("stream error") {
            answer.push_str(&t);
        }
    }
    assert!(
        answer.contains("Anthropic"),
        "answer not grounded in web result: {answer}"
    );
}

/// Sliding-window history: a follow-up with a pronoun must resolve against the
/// injected prior turns.
#[tokio::test]
#[ignore]
async fn history_window_resolves_followup_reference() {
    let client = OllamaClient::new("http://127.0.0.1:11434");
    assert!(client.is_up().await, "Ollama not reachable");

    let messages = vec![
        Message::user("I pay monthly for the Claude Code Pro subscription from Anthropic."),
        Message::assistant("Understood — you have the Claude Pro plan which includes Claude Code."),
        Message::user("What other options do I have?"),
    ];
    let stream = client.chat_stream_with_tools(MODEL.into(), messages, vec![]);
    tokio::pin!(stream);
    let mut answer = String::new();
    while let Some(ev) = stream.next().await {
        if let ChatStreamEvent::Delta(t) = ev.expect("stream error") {
            answer.push_str(&t);
        }
    }
    let a = answer.to_lowercase();
    assert!(
        a.contains("claude") || a.contains("anthropic") || a.contains("subscription") || a.contains("plan"),
        "follow-up not resolved from history: {answer}"
    );
}
