use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use basert_connector::{BaseRtClient, ChatStreamEvent, Message, ToolDef};
use futures_util::StreamExt;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<serde_json::Value>>>);

async fn spawn_server(response: Response) -> (String, Capture) {
    let capture = Capture::default();
    let app = Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .route(
            "/v1/models",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers.get("authorization").unwrap(), "Bearer test-key");
                axum::Json(json!({
                    "data": [{"id": "basecompute/Qwen3-4B-Instruct-2507"}]
                }))
            }),
        )
        .route(
            "/v1/chat/completions",
            post({
                let response = Arc::new(Mutex::new(Some(response)));
                move |State(capture): State<Capture>, headers: HeaderMap, request: Request<Body>| {
                    let response = response.clone();
                    async move {
                        assert_eq!(headers.get("authorization").unwrap(), "Bearer test-key");
                        let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
                            .await
                            .unwrap();
                        capture
                            .0
                            .lock()
                            .unwrap()
                            .push(serde_json::from_slice(&bytes).unwrap());
                        response.lock().unwrap().take().unwrap()
                    }
                }
            }),
        )
        .with_state(capture.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/v1"), capture)
}

#[tokio::test]
async fn authenticates_health_and_lists_models() {
    let response = StatusCode::INTERNAL_SERVER_ERROR.into_response();
    let (base_url, _) = spawn_server(response).await;
    let client = BaseRtClient::new(base_url, "test-key");

    assert!(client.is_up().await);
    assert_eq!(
        client.models().await.unwrap(),
        vec!["basecompute/Qwen3-4B-Instruct-2507"]
    );
}

#[tokio::test]
async fn streams_content_and_reassembles_fragmented_tool_calls() {
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Ahoj \"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"mail_\",\"arguments\":\"{\\\"send\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"search\",\"arguments\":\"er\\\":\\\"tomas\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let response = Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .body(Body::from(sse))
        .unwrap();
    let (base_url, capture) = spawn_server(response).await;
    let client = BaseRtClient::new(base_url, "test-key");
    let tools = vec![ToolDef::function(
        "mail_search",
        "Search mail",
        json!({"type":"object"}),
    )];

    let stream = client.chat_stream_with_tools(
        "basecompute/Qwen3-4B-Instruct-2507".into(),
        vec![Message::user("Nájdi mail")],
        tools,
    );
    tokio::pin!(stream);
    let mut content = String::new();
    let mut calls = Vec::new();
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            ChatStreamEvent::Delta(delta) => content.push_str(&delta),
            ChatStreamEvent::ToolCalls(found) => calls.extend(found),
        }
    }

    assert_eq!(content, "Ahoj ");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].function.name, "mail_search");
    assert_eq!(calls[0].function.arguments, json!({"sender":"tomas"}));

    let request = &capture.0.lock().unwrap()[0];
    assert_eq!(request["stream"], true);
    assert_eq!(request["model"], "basecompute/Qwen3-4B-Instruct-2507");
}

#[tokio::test]
async fn serializes_openai_tool_result_ids() {
    let response = Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .body(Body::from("data: [DONE]\n\n"))
        .unwrap();
    let (base_url, capture) = spawn_server(response).await;
    let client = BaseRtClient::new(base_url, "test-key");
    let messages = vec![Message::tool_result("call_42", "mail_search", "[]")];
    let stream = client.chat_stream("model".into(), messages);
    tokio::pin!(stream);
    while stream.next().await.is_some() {}

    let request = &capture.0.lock().unwrap()[0];
    assert_eq!(request["messages"][0]["role"], "tool");
    assert_eq!(request["messages"][0]["tool_call_id"], "call_42");
    assert!(request["messages"][0].get("name").is_none());
}

#[tokio::test]
async fn surfaces_openai_error_payloads() {
    let response = Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"error":{"message":"context is too long","code":"context_length_exceeded"}}"#,
        ))
        .unwrap();
    let (base_url, _) = spawn_server(response).await;
    let client = BaseRtClient::new(base_url, "test-key");
    let stream = client.chat_stream("model".into(), vec![Message::user("hello")]);
    tokio::pin!(stream);

    let error = stream.next().await.unwrap().unwrap_err().to_string();
    assert!(error.contains("context is too long"), "{error}");
}
