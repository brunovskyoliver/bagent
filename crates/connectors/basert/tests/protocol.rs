use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use basert_connector::{
    classify_basert_runtime_fault, BaseRtClient, BaseRtCompletionError, BaseRtRuntimeFault,
    ChatStreamEvent, Message, ModelLoadRequest, ModelReadiness, ToolDef,
};
use futures_util::StreamExt;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::{
    net::TcpListener,
    sync::{oneshot, Notify},
    time::{timeout, Duration},
};

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
async fn typed_model_lifecycle_loads_checks_readiness_and_unloads() {
    #[derive(Clone, Default)]
    struct LifecycleCapture(Arc<Mutex<Vec<(String, serde_json::Value)>>>);

    async fn capture_request(
        State(capture): State<LifecycleCapture>,
        headers: HeaderMap,
        request: Request<Body>,
    ) -> impl IntoResponse {
        assert_eq!(headers.get("authorization").unwrap(), "Bearer test-key");
        let path = request.uri().path().to_string();
        let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        capture.0.lock().unwrap().push((path, body));
        StatusCode::OK
    }

    let capture = LifecycleCapture::default();
    let app = Router::new()
        .route("/v1/models/load", post(capture_request))
        .route("/v1/models/unload", post(capture_request))
        .route(
            "/v1/models",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers.get("authorization").unwrap(), "Bearer test-key");
                axum::Json(json!({
                    "data": [{
                        "id": "basecompute/Qwen3.6-35B-A3B",
                        "loaded": true
                    }]
                }))
            }),
        )
        .with_state(capture.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = BaseRtClient::new(format!("http://{address}/v1"), "test-key");
    let readiness = client
        .load_model(&ModelLoadRequest {
            id: "basecompute/Qwen3.6-35B-A3B".into(),
            path: "/models/qwen35.base".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        readiness,
        ModelReadiness {
            id: "basecompute/Qwen3.6-35B-A3B".into(),
            known: true,
            loaded: true,
        }
    );
    client
        .unload_model("basecompute/Qwen3.6-35B-A3B")
        .await
        .unwrap();

    let requests = capture.0.lock().unwrap();
    assert_eq!(
        requests.as_slice(),
        [
            (
                "/v1/models/load".into(),
                json!({"path": "/models/qwen35.base"})
            ),
            (
                "/v1/models/unload".into(),
                json!({"model": "basecompute/Qwen3.6-35B-A3B"})
            )
        ]
    );
}

#[tokio::test]
async fn model_load_and_unload_wait_for_an_active_legacy_completion() {
    #[derive(Clone)]
    struct CoordinationState {
        chat_started: Arc<Notify>,
        release_chat: Arc<Notify>,
        lifecycle_seen: Arc<Notify>,
    }

    let state = CoordinationState {
        chat_started: Arc::new(Notify::new()),
        release_chat: Arc::new(Notify::new()),
        lifecycle_seen: Arc::new(Notify::new()),
    };
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(
                |State(state): State<CoordinationState>, _request: Request<Body>| async move {
                    state.chat_started.notify_one();
                    state.release_chat.notified().await;
                    Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "text/event-stream")
                        .body(Body::from(
                            "data: {\"choices\":[{\"delta\":{\"content\":\"completed\"}}]}\n\n\
                             data: [DONE]\n\n",
                        ))
                        .unwrap()
                },
            ),
        )
        .route(
            "/v1/models/unload",
            post(
                |State(state): State<CoordinationState>, _request: Request<Body>| async move {
                    state.lifecycle_seen.notify_one();
                    StatusCode::OK
                },
            ),
        )
        .route(
            "/v1/models/load",
            post(
                |State(state): State<CoordinationState>, _request: Request<Body>| async move {
                    state.lifecycle_seen.notify_one();
                    StatusCode::OK
                },
            ),
        )
        .route(
            "/v1/models",
            get(|| async {
                axum::Json(json!({
                    "data": [{
                        "id": "basecompute/Qwen3.6-35B-A3B",
                        "loaded": true
                    }]
                }))
            }),
        )
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = BaseRtClient::new(format!("http://{address}/v1"), "test-key");
    let chat_client = client.clone();
    let (chat_done_tx, chat_done_rx) = oneshot::channel();
    tokio::spawn(async move {
        let stream = chat_client.chat_stream(
            "basecompute/Qwen3-4B-Instruct-2507".into(),
            vec![Message::user("legacy request")],
        );
        tokio::pin!(stream);
        let mut completed = String::new();
        while let Some(part) = stream.next().await {
            completed.push_str(&part.unwrap());
        }
        let _ = chat_done_tx.send(completed);
    });
    state.chat_started.notified().await;

    let unload_client = client.clone();
    let (unload_done_tx, unload_done_rx) = oneshot::channel();
    tokio::spawn(async move {
        let result = unload_client
            .unload_model("basecompute/Qwen3.6-35B-A3B")
            .await;
        let _ = unload_done_tx.send(result);
    });

    assert!(
        timeout(Duration::from_millis(50), state.lifecycle_seen.notified())
            .await
            .is_err(),
        "unload reached BaseRT while the legacy completion was active"
    );
    state.release_chat.notify_one();
    assert_eq!(chat_done_rx.await.unwrap(), "completed");
    unload_done_rx.await.unwrap().unwrap();
    timeout(Duration::from_secs(1), state.lifecycle_seen.notified())
        .await
        .expect("unload should proceed after completion releases its lease");

    let chat_client = client.clone();
    let (chat_done_tx, chat_done_rx) = oneshot::channel();
    tokio::spawn(async move {
        let stream = chat_client.chat_stream(
            "basecompute/Qwen3-4B-Instruct-2507".into(),
            vec![Message::user("legacy request")],
        );
        tokio::pin!(stream);
        let mut completed = String::new();
        while let Some(part) = stream.next().await {
            completed.push_str(&part.unwrap());
        }
        let _ = chat_done_tx.send(completed);
    });
    state.chat_started.notified().await;

    let load_client = client.clone();
    let (load_done_tx, load_done_rx) = oneshot::channel();
    tokio::spawn(async move {
        let result = load_client
            .load_model(&ModelLoadRequest {
                id: "basecompute/Qwen3.6-35B-A3B".into(),
                path: "/models/qwen35.base".into(),
            })
            .await;
        let _ = load_done_tx.send(result);
    });
    assert!(
        timeout(Duration::from_millis(50), state.lifecycle_seen.notified())
            .await
            .is_err(),
        "load reached BaseRT while the legacy completion was active"
    );
    state.release_chat.notify_one();
    assert_eq!(chat_done_rx.await.unwrap(), "completed");
    assert!(load_done_rx.await.unwrap().unwrap().loaded);
    timeout(Duration::from_secs(1), state.lifecycle_seen.notified())
        .await
        .expect("load should proceed after completion releases its lease");
}

#[tokio::test]
async fn bounded_chat_completion_is_non_streamed_and_uses_requested_limit() {
    let response = (
        StatusCode::OK,
        axum::Json(json!({
            "choices": [{"message": {"content": "Held response"}}]
        })),
    )
        .into_response();
    let (base_url, capture) = spawn_server(response).await;
    let client = BaseRtClient::new(base_url, "test-key");

    let response = client
        .chat_complete_bounded(
            "configured-4b",
            vec![Message::system("system"), Message::user("user")],
            0.2,
            512,
        )
        .await
        .unwrap();

    assert_eq!(response, "Held response");
    let request = &capture.0.lock().unwrap()[0];
    assert_eq!(request["stream"], false);
    assert_eq!(request["max_tokens"], 512);
    assert_eq!(request["tools"], serde_json::Value::Null);
    assert_eq!(
        request["chat_template_kwargs"]["enable_thinking"],
        serde_json::Value::Bool(false)
    );
    assert_eq!(request["messages"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn bounded_chat_completion_rejects_empty_model_output_as_unavailable() {
    let response = (
        StatusCode::OK,
        axum::Json(json!({
            "choices": [{"message": {"content": ""}, "finish_reason": "stop"}]
        })),
    )
        .into_response();
    let (base_url, _) = spawn_server(response).await;
    let client = BaseRtClient::new(base_url, "test-key");

    let error = client
        .chat_complete_bounded(
            "configured-model",
            vec![Message::system("system"), Message::user("user")],
            0.1,
            512,
        )
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(error, "BaseRT returned an empty completion");
}

#[tokio::test]
async fn bounded_chat_completion_reports_output_cap_truncation() {
    let response = (
        StatusCode::OK,
        axum::Json(json!({
            "choices": [{"message": {"content": "partial"}, "finish_reason": "length"}]
        })),
    )
        .into_response();
    let (base_url, _) = spawn_server(response).await;
    let client = BaseRtClient::new(base_url, "test-key");

    let error = client
        .chat_complete_bounded(
            "configured-model",
            vec![Message::system("system"), Message::user("user")],
            0.1,
            256,
        )
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(error, "BaseRT generation truncated at output cap");
}

#[tokio::test]
async fn metal_log_fault_overrides_http_error_and_suppresses_the_next_request() {
    let log_path = std::env::temp_dir().join(format!(
        "bagent-basert-metal-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app = Router::new().route(
        "/v1/chat/completions",
        post({
            let log_path = log_path.clone();
            let requests = requests.clone();
            move || {
                let log_path = log_path.clone();
                let requests = requests.clone();
                async move {
                    requests.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    tokio::fs::write(
                        log_path,
                        "[baseRT][metal] command buffer failed: Insufficient Memory \
                         (00000008:kIOGPUCommandBufferCallbackErrorOutOfMemory)",
                    )
                    .await
                    .unwrap();
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = BaseRtClient::new(format!("http://{address}/v1"), "test-key")
        .with_runtime_log_path(&log_path);

    let first = client
        .chat_complete_bounded(
            "configured-model",
            vec![Message::system("system"), Message::user("user")],
            0.1,
            256,
        )
        .await
        .unwrap_err();
    assert_eq!(
        first.downcast_ref::<BaseRtCompletionError>(),
        Some(&BaseRtCompletionError::RuntimeFault(
            BaseRtRuntimeFault::MetalOutOfMemory
        ))
    );

    let second = client
        .chat_complete_bounded(
            "configured-model",
            vec![Message::system("system"), Message::user("user")],
            0.1,
            256,
        )
        .await
        .unwrap_err();
    assert_eq!(
        second.downcast_ref::<BaseRtCompletionError>(),
        Some(&BaseRtCompletionError::RuntimeFault(
            BaseRtRuntimeFault::MetalOutOfMemory
        ))
    );
    assert_eq!(
        requests.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the poisoned process must not receive a second request"
    );
    let _ = tokio::fs::remove_file(log_path).await;
}

#[tokio::test]
async fn metal_fault_is_detected_after_same_or_larger_log_rotation() {
    let log_path = std::env::temp_dir().join(format!(
        "bagent-basert-rotation-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let rotated_path = log_path.with_extension("old");
    tokio::fs::write(&log_path, "x".repeat(128)).await.unwrap();
    let client =
        BaseRtClient::new("http://127.0.0.1:1/v1", "test-key").with_runtime_log_path(&log_path);
    let checkpoint = client.runtime_log_checkpoint().await;
    tokio::fs::rename(&log_path, &rotated_path).await.unwrap();
    tokio::fs::write(
        &log_path,
        format!(
            "[baseRT][metal] command buffer failed: device was lost{}",
            "y".repeat(128)
        ),
    )
    .await
    .unwrap();

    assert_eq!(
        client
            .detect_runtime_fault_since(checkpoint, Duration::ZERO)
            .await,
        Some(BaseRtRuntimeFault::MetalDevice)
    );
    let _ = tokio::fs::remove_file(log_path).await;
    let _ = tokio::fs::remove_file(rotated_path).await;
}

#[test]
fn normalizes_only_poisoning_metal_log_signatures() {
    assert_eq!(
        classify_basert_runtime_fault(
            "[baseRT][metal] command buffer failed: Insufficient Memory \
             (00000008:kIOGPUCommandBufferCallbackErrorOutOfMemory)"
        ),
        Some(BaseRtRuntimeFault::MetalOutOfMemory)
    );
    assert_eq!(
        classify_basert_runtime_fault("[baseRT][metal] device was lost"),
        Some(BaseRtRuntimeFault::MetalDevice)
    );
    assert_eq!(
        classify_basert_runtime_fault("[baseRT][metal] command buffer failed: internal error"),
        Some(BaseRtRuntimeFault::MetalCommandBuffer)
    );
    assert_eq!(
        classify_basert_runtime_fault("Model loaded via API; prompt=261 completion=1"),
        None
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

#[tokio::test]
async fn preserves_utf8_split_across_transport_chunks() {
    let payload = "data: {\"choices\":[{\"delta\":{\"content\":\"Dobrý deň 👋\"}}]}\n\n\
                   data: [DONE]\n\n";
    let bytes = payload.as_bytes();
    let split = payload.find('ý').unwrap() + 1;
    let chunks = vec![
        Ok::<_, std::convert::Infallible>(Bytes::copy_from_slice(&bytes[..split])),
        Ok(Bytes::copy_from_slice(&bytes[split..])),
    ];
    let response = Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(futures_util::stream::iter(chunks)))
        .unwrap();
    let (base_url, _) = spawn_server(response).await;
    let client = BaseRtClient::new(base_url, "test-key");
    let stream = client.chat_stream("model".into(), vec![Message::user("hello")]);
    tokio::pin!(stream);
    let mut content = String::new();
    while let Some(delta) = stream.next().await {
        content.push_str(&delta.unwrap());
    }
    assert_eq!(content, "Dobrý deň 👋");
}
