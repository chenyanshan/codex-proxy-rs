use super::*;

#[tokio::test]
async fn probe_logs_request_headers_response_fields_state_and_redacts_nested_secrets() {
    let logs = crate::support::capture_logs();
    let proxy = MockServer::start().await;
    let (_, _, manager) = fixture(Some(&proxy.uri())).await;
    for model in SESSION_KEEPALIVE_MODELS {
        mock_model(&proxy, "acct_a", model, ResponseTemplate::new(200)
            .insert_header("x-codex-turn-state", state("diagnostic-state"))
            .insert_header("x-request-id", "upstream-request-123")
            .insert_header("set-cookie", "private-cookie-value")
            .set_body_raw("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_diagnostic\",\"status\":\"completed\",\"usage\":{\"total_tokens\":7},\"refresh_token\":\"nested-secret-value\"}}\n\n", "text/event-stream")).await;
    }
    let result = manager
        .refresh(&ProviderAccountId::new("acct_a").unwrap())
        .await
        .unwrap();
    assert!(result.models.iter().all(|item| item.error.is_none()));
    let events: Vec<_> = logs
        .json_events()
        .into_iter()
        .filter(|event| event["target"] == "session_keepalive")
        .collect();
    let output = serde_json::to_string(&events).unwrap();
    for field in [
        "diagnostic-state",
        "upstream-request-123",
        "resp_diagnostic",
        "total_tokens",
        "response_headers",
        "response_body",
        "probe_id",
        "elapsedMs",
        "content-encoding",
        "hi",
    ] {
        assert!(output.contains(field), "missing {field}");
    }
    for secret in [
        "private-cookie-value",
        "nested-secret-value",
        "Bearer acct_a",
    ] {
        assert!(!output.contains(secret), "leaked secret");
    }
    assert!(output.contains("[REDACTED]"));
}

#[tokio::test]
async fn upstream_rejection_reports_status_and_probe_id_without_authentication() {
    let logs = crate::support::capture_logs();
    let proxy = MockServer::start().await;
    let (_, _, manager) = fixture(Some(&proxy.uri())).await;
    for model in SESSION_KEEPALIVE_MODELS {
        mock_model(
            &proxy,
            "acct_a",
            model,
            ResponseTemplate::new(403).set_body_json(
                json!({"error":{"message":"Rejected Bearer acct_a"},"request_id":"denied-123"}),
            ),
        )
        .await;
    }
    let id = ProviderAccountId::new("acct_a").unwrap();
    let refresh = manager.refresh(&id);
    tokio::pin!(refresh);
    let inspect = async {
        loop {
            let errors: Vec<_> = logs
                .json_events()
                .into_iter()
                .filter_map(|event| event["fields"]["error"].as_str().map(str::to_owned))
                .filter(|error| error.contains("HTTP 403"))
                .collect();
            if errors.len() >= 2 {
                return errors;
            }
            tokio::task::yield_now().await;
        }
    };
    let errors = tokio::select! {
        _ = &mut refresh => panic!("失败模型应该持续重试"),
        errors = tokio::time::timeout(Duration::from_secs(3), inspect) => errors.unwrap(),
    };
    for error in errors {
        assert!(error.contains("重写"));
        assert!(error.contains("[REDACTED]"));
        assert!(!error.contains("acct_a"));
    }
}
