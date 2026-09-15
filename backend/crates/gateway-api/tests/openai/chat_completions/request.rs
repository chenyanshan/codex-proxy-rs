use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use futures::future::BoxFuture;
use gateway_core::engine::execution::{
    AuthenticatedClient, ClientAuthenticationError, ExecutionService, StartExecution,
    StartProviderExecution, StartedExecution,
};
use gateway_core::error::{GatewayError, GatewayErrorKind};
use gateway_core::operation::Operation;
use gateway_core::routing::PublicModelId;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::openai::{api_router, authenticated_client};

#[derive(Default)]
struct Capture {
    request: Mutex<Option<(Value, Value, String)>>,
}

impl ExecutionService for Capture {
    fn authenticate(
        &self,
        plaintext: &str,
    ) -> Result<AuthenticatedClient, ClientAuthenticationError> {
        if plaintext == "sk_chat_request_test" {
            Ok(authenticated_client(plaintext))
        } else {
            Err(ClientAuthenticationError::InvalidKey)
        }
    }

    fn public_models(&self, _: &AuthenticatedClient) -> Vec<PublicModelId> {
        vec![PublicModelId::new("model-a").expect("model")]
    }

    fn contains_public_model(&self, _: &AuthenticatedClient, model: &PublicModelId) -> bool {
        model.as_str() == "model-a"
    }

    fn start(
        &self,
        request: StartExecution,
    ) -> BoxFuture<'_, Result<StartedExecution, GatewayError>> {
        Box::pin(async move {
            let Operation::Generate(generation) = request.operation else {
                panic!("Chat must use the shared Generate operation")
            };
            let payload = generation.protocol_payload();
            *self.request.lock().expect("capture") = Some((
                Value::Object(payload.body().clone()),
                Value::Object(payload.context().clone()),
                request.metadata.endpoint,
            ));
            Err(GatewayError::new(
                GatewayErrorKind::Internal,
                "request captured",
            ))
        })
    }

    fn start_provider_endpoint(
        &self,
        _: StartProviderExecution,
    ) -> BoxFuture<'_, Result<StartedExecution, GatewayError>> {
        Box::pin(async { panic!("Chat must not bypass shared Generate execution") })
    }
}

async fn send(body: Vec<u8>, headers: HeaderMap) -> (StatusCode, Value, Arc<Capture>) {
    let capture = Arc::new(Capture::default());
    let router = api_router(capture.clone()).await;
    let mut request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer sk_chat_request_test")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("request");
    request.headers_mut().extend(headers);
    let response = router.oneshot(request).await.expect("response");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&body).expect("JSON response"),
        capture,
    )
}

async fn converted(request: Value) -> Value {
    let (status, _, capture) = send(
        serde_json::to_vec(&request).expect("JSON"),
        HeaderMap::new(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "capture terminates before execution"
    );
    let request = capture
        .request
        .lock()
        .expect("capture")
        .take()
        .expect("started execution");
    assert_eq!(request.2, "/v1/chat/completions");
    request.0
}

#[tokio::test]
async fn preserves_message_order_images_and_opaque_function_arguments() {
    let body = converted(json!({
        "model":"model-a",
        "messages":[
            {"role":"system","content":"first"},
            {"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"data:image/png;base64,aGVsbG8=","detail":"high"}}]},
            {"role":"developer","content":"later instruction"},
            {"role":"assistant","content":"checking","tool_calls":[{"id":"call exact/1","type":"function","function":{"name":"lookup","arguments":"{incomplete"}}]},
            {"role":"tool","tool_call_id":"call exact/1","content":[{"type":"text","text":"part one"},{"type":"text","text":"part two"}]},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call2","type":"function","function":{"name":"lookup","arguments":""}}]},
            {"role":"tool","tool_call_id":"call2","content":"done"}
        ]
    })).await;
    assert_eq!(body["stream"], false);
    assert_eq!(body["store"], false);
    assert!(body.get("messages").is_none());
    assert_eq!(
        body["input"],
        json!([
            {"role":"system","content":"first"},
            {"role":"user","content":[{"type":"input_text","text":"look"},{"type":"input_image","image_url":"data:image/png;base64,aGVsbG8=","detail":"high"}]},
            {"role":"developer","content":"later instruction"},
            {"role":"assistant","content":"checking"},
            {"type":"function_call","call_id":"call exact/1","name":"lookup","arguments":"{incomplete"},
            {"type":"function_call_output","call_id":"call exact/1","output":[{"type":"input_text","text":"part one"},{"type":"input_text","text":"part two"}]},
            {"type":"function_call","call_id":"call2","name":"lookup","arguments":""},
            {"type":"function_call_output","call_id":"call2","output":"done"}
        ])
    );
}

#[tokio::test]
async fn maps_function_tools_and_structured_output_without_rewriting_schema() {
    let schema = json!({"type":"object","properties":{"x":{"type":"string","future_schema_keyword":true}},"required":["x"],"additionalProperties":false});
    let body = converted(json!({
        "model":"model-a","messages":[{"role":"user","content":"hello"}],
        "tools":[{"type":"function","function":{"name":"lookup","description":"find","parameters":schema}}],
        "tool_choice":{"type":"function","function":{"name":"lookup"}},"parallel_tool_calls":false,
        "response_format":{"type":"json_schema","json_schema":{"name":"answer","description":"result","schema":schema,"strict":true}},
        "verbosity":"low","top_p":0.5
    })).await;
    assert_eq!(
        body["tools"],
        json!([{"type":"function","name":"lookup","description":"find","parameters":schema,"strict":false}])
    );
    assert_eq!(
        body["tool_choice"],
        json!({"type":"function","name":"lookup"})
    );
    assert_eq!(body["parallel_tool_calls"], false);
    assert_eq!(
        body["text"],
        json!({"format":{"type":"json_schema","name":"answer","description":"result","schema":schema,"strict":true},"verbosity":"low"})
    );
    assert_eq!(body["top_p"], 0.5);
}

#[tokio::test]
async fn accepts_optional_nulls_and_neutral_defaults() {
    let body = converted(json!({
        "model":"model-a","messages":[{"role":"user","content":"hi","name":null}],
        "stream":null,"stream_options":null,"n":null,"store":false,"logprobs":false,
        "presence_penalty":0,"frequency_penalty":0.0,"logit_bias":{},"metadata":{},
        "service_tier":"auto","modalities":["text"],"temperature":null,"reasoning_effort":null,
        "max_tokens":null,"max_completion_tokens":null,"tools":null,"response_format":null
    }))
    .await;
    assert_eq!(
        body,
        json!({"model":"model-a","stream":false,"store":false,"input":[{"role":"user","content":"hi"}]})
    );
}

#[tokio::test]
async fn preserves_assistant_text_parts_and_clipped_tool_history() {
    let body = converted(json!({
        "model":"model-a", "n":1,
        "messages":[
            {"role":"assistant","content":[{"type":"text","text":"first"},{"type":"text","text":"second"}],"refusal":null},
            {"role":"tool","tool_call_id":"history-call","content":"result"},
            {"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.com/image.png"}}]}
        ], "response_format":{"type":"json_object"}
    })).await;
    assert_eq!(
        body["input"][0],
        json!({"role":"assistant","content":[{"type":"input_text","text":"first"},{"type":"input_text","text":"second"}]})
    );
    assert_eq!(body["input"][1]["call_id"], "history-call");
    assert_eq!(
        body["input"][2]["content"][0]["image_url"],
        "https://example.com/image.png"
    );
    assert_eq!(body["text"]["format"], json!({"type":"json_object"}));
}

#[tokio::test]
async fn rejects_unsupported_controls_without_starting_execution() {
    for (field, value) in [
        ("temperature", json!(1)),
        ("max_tokens", json!(10)),
        ("max_completion_tokens", json!(10)),
        ("reasoning_effort", json!("high")),
        ("service_tier", json!("default")),
        ("n", json!(2)),
        ("store", json!(true)),
        ("logprobs", json!(true)),
        ("metadata", json!({"private":"secret"})),
        ("stop", json!(["END"])),
    ] {
        let mut request = json!({"model":"model-a","messages":[{"role":"user","content":"hi"}]});
        request[field] = value;
        let (status, body, capture) = send(
            serde_json::to_vec(&request).expect("JSON"),
            HeaderMap::new(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {body}");
        assert_eq!(body["error"]["param"], field);
        assert_eq!(body["error"]["code"], "unsupported_parameter");
        assert!(capture.request.lock().expect("capture").is_none());
        assert!(!body.to_string().contains("secret"));
    }
}

#[tokio::test]
async fn rejects_unknown_fields_and_malformed_nested_values() {
    for (extra, param) in [
        (json!({"unknown_future":null}), "unknown_future"),
        (
            json!({"messages":[{"role":"user","content":[{"type":"input_audio","input_audio":{"data":"secret"}}]}]}),
            "messages[0].content[0].type",
        ),
        (
            json!({"tool_choice":{"type":"function","function":{"name":"undeclared"}}}),
            "tool_choice.function.name",
        ),
        (
            json!({"stream_options":{"include_usage":true}}),
            "stream_options",
        ),
        (
            json!({"stream":true,"stream_options":{"include_usage":"true"}}),
            "stream_options.include_usage",
        ),
        (json!({"tool_choice":"required","tools":[]}), "tool_choice"),
        (
            json!({"messages":[{"role":"tool","tool_call_id":"call"}]}),
            "messages[0].content",
        ),
        (
            json!({"response_format":{"type":"json_schema","json_schema":{"name":"answer","schema":{},"strict":"true"}}}),
            "response_format.json_schema.strict",
        ),
        (
            json!({"stream":true,"stream_options":{"include_obfuscation":true}}),
            "stream_options.include_obfuscation",
        ),
        (
            json!({"messages":[{"role":"assistant","content":null}]}),
            "messages[0].content",
        ),
        (
            json!({"messages":[{"role":"assistant","refusal":"private refusal"}]}),
            "messages[0].refusal",
        ),
        (
            json!({"messages":[{"role":"assistant","content":[{"type":"refusal","refusal":"private refusal"}]}]}),
            "messages[0].content[0].type",
        ),
    ] {
        let mut request = json!({"model":"model-a","messages":[{"role":"user","content":"hi"}]});
        request
            .as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        let (status, body, capture) = send(
            serde_json::to_vec(&request).expect("JSON"),
            HeaderMap::new(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"]["param"], param);
        assert!(capture.request.lock().expect("capture").is_none());
    }
}

#[tokio::test]
async fn decompresses_once_and_preserves_session_headers() {
    let request = br#"{"model":"model-a","messages":[{"role":"user","content":"hi"}]}"#;
    for encoding in ["gzip", "deflate", "zstd"] {
        let compressed = match encoding {
            "gzip" => {
                let mut encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(request).expect("encode");
                encoder.finish().expect("finish")
            }
            "deflate" => {
                let mut encoder =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(request).expect("encode");
                encoder.finish().expect("finish")
            }
            _ => zstd::stream::encode_all(request.as_slice(), 1).expect("encode"),
        };
        let mut headers = HeaderMap::new();
        headers.insert("content-encoding", encoding.parse().expect("encoding"));
        headers.insert("session-id", "chat-session".parse().expect("session"));
        headers.insert("x-codex-turn-state", "chat-turn".parse().expect("turn"));
        let (status, _, capture) = send(compressed, headers).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let captured = capture
            .request
            .lock()
            .expect("capture")
            .take()
            .expect("decoded");
        assert_eq!(captured.0["input"][0]["content"], "hi");
        assert_eq!(captured.1["session_id"], "chat-session");
        assert_eq!(captured.1["turn_state"], "chat-turn");
    }
}
