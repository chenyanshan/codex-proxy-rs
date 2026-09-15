use super::*;

#[tokio::test]
async fn json_terminal_projects_text_refusal_tools_and_usage_without_private_reasoning() {
    let output = json!([
        {"type":"reasoning","encrypted_content":"private-encrypted","summary":[{"text":"private-reasoning"}]},
        {"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"},{"type":"refusal","refusal":"declined"}]},
        {"type":"function_call","call_id":"call_a","name":"lookup","arguments":"{\"city\":\"北京\"}"}
    ]);
    let (status, value) = json_response(vec![completed(output)]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["object"], "chat.completion");
    assert_eq!(value["id"], "chatcmpl-req_chat_test");
    assert_eq!(value["created"], 1_700_000_000_u64);
    assert_eq!(value["model"], "public-chat-model");
    assert_eq!(value["choices"][0]["message"]["content"], "answer");
    assert_eq!(value["choices"][0]["message"]["refusal"], "declined");
    assert_eq!(
        value["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
        "{\"city\":\"北京\"}"
    );
    assert_eq!(value["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(
        value["usage"],
        json!({"prompt_tokens":10,"completion_tokens":4,"total_tokens":14,"prompt_tokens_details":{"cached_tokens":3},"completion_tokens_details":{"reasoning_tokens":2}})
    );
    assert!(!value.to_string().contains("private-"));
}

#[tokio::test]
async fn delta_done_and_terminal_snapshots_do_not_duplicate_text() {
    let events = vec![
        delta("你"),
        delta("好"),
        wire(
            "response.output_text.done",
            json!({"output_index":0,"content_index":0,"text":"你好!"}),
        ),
        completed(json!([message("你好!")])),
    ];
    let (status, value) = json_response(events).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["choices"][0]["message"]["content"], "你好!");
}

#[tokio::test]
async fn done_only_and_missing_terminal_output_retain_observed_text() {
    let (status, value) = json_response(vec![
        wire(
            "response.output_text.done",
            json!({"output_index":0,"content_index":0,"text":"done-only"}),
        ),
        wire(
            "response.completed",
            json!({"response":{"status":"completed"}}),
        ),
    ])
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["choices"][0]["message"]["content"], "done-only");
    assert!(value.get("usage").is_none());
}

#[tokio::test]
async fn incomplete_reasons_are_explicit() {
    for (reason, expected) in [
        ("max_output_tokens", "length"),
        ("content_filter", "content_filter"),
    ] {
        let (status, value) = json_response(vec![wire("response.incomplete", json!({"response":{"status":"incomplete","output":[message("partial")],"incomplete_details":{"reason":reason}}}))]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["choices"][0]["finish_reason"], expected);
    }
    let (status, _) = json_response(vec![wire("response.incomplete", json!({"response":{"status":"incomplete","output":[],"incomplete_details":{"reason":"unknown"}}}))]).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn malformed_terminal_and_conflicting_snapshots_fail_before_json_commit() {
    for events in [
        vec![delta("already"), completed(json!([message("different")]))],
        vec![wire(
            "response.completed",
            json!({"response":{"status":"failed","output":[]}}),
        )],
        vec![wire(
            "response.completed",
            json!({"response":{"status":"completed","output":null}}),
        )],
        vec![completed(
            json!([{"type":"function_call","call_id":"c","name":"f","arguments":42}]),
        )],
        vec![completed(
            json!([{"type":"message","role":"user","content":[]}]),
        )],
        vec![wire(
            "response.completed",
            json!({"response":{"status":"completed","output":[],"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":99}}}),
        )],
    ] {
        let session = Session::json(events);
        let trace = session.trace.clone();
        let response = call(session, false, false).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !trace
                .events()
                .iter()
                .any(|event| event.starts_with("commit:"))
        );
        assert!(trace.events().contains(&"cancel".to_owned()));
        assert_eq!(
            trace
                .events()
                .iter()
                .filter(|event| *event == "finalize")
                .count(),
            1
        );
    }
}
