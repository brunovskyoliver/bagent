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
