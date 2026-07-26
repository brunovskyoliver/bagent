//! Live BaseRT regressions.
//!
//! Start the bagent BaseRT service on port 8082, then run:
//! `cargo test -p basert-connector --test live -- --ignored`

use basert_connector::{
    BaseRtClient, ChatStreamEvent, Message, ToolDef, DEFAULT_API_KEY, DEFAULT_BASE_URL,
    DEFAULT_CHAT_MODEL,
};
use futures_util::StreamExt;

#[tokio::test]
#[ignore = "requires bagent BaseRT on port 8082"]
async fn slovak_diacritics_are_preserved() {
    let client = BaseRtClient::new(DEFAULT_BASE_URL, DEFAULT_API_KEY);
    assert!(client.is_up().await, "BaseRT is not reachable on port 8082");
    let stream = client.chat_stream(
        DEFAULT_CHAT_MODEL.to_string(),
        vec![Message::user(
            "Napíš jednu krátku slovenskú vetu so slovami faktúra a splatnosť.",
        )],
    );
    tokio::pin!(stream);
    let mut answer = String::new();
    while let Some(chunk) = stream.next().await {
        answer.push_str(&chunk.expect("stream error"));
    }
    let lower = answer.to_lowercase();
    assert!(lower.contains("faktúr"), "{answer}");
    assert!(lower.contains("splatnosť"), "{answer}");
}

#[tokio::test]
#[ignore = "requires bagent BaseRT on port 8082"]
async fn native_tool_call_round_trip_is_openai_compatible() {
    let client = BaseRtClient::new(DEFAULT_BASE_URL, DEFAULT_API_KEY);
    let tool = ToolDef::function(
        "mail_search",
        "Search the user's mail. Always use this tool for mail requests.",
        serde_json::json!({
            "type": "object",
            "properties": {"sender": {"type": "string"}},
            "required": ["sender"]
        }),
    );
    let stream = client.chat_stream_with_tools(
        DEFAULT_CHAT_MODEL.to_string(),
        vec![Message::user(
            "Nájdi posledný email od tomas.juricek@novem.sk.",
        )],
        vec![tool],
    );
    tokio::pin!(stream);
    let mut calls = Vec::new();
    while let Some(event) = stream.next().await {
        if let ChatStreamEvent::ToolCalls(found) = event.expect("stream error") {
            calls.extend(found);
        }
    }
    assert!(
        calls.iter().any(|call| {
            call.function.name == "mail_search"
                && call
                    .function
                    .arguments
                    .to_string()
                    .contains("tomas.juricek")
        }),
        "model did not emit the expected mail_search call: {calls:?}"
    );
}
