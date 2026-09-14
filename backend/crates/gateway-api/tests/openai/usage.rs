use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

fn execution(suspended: bool) -> Arc<dyn ExecutionService> {
    let directory = Arc::new(RuntimeAccountDirectory::default());
    let policies = [
        ("first", "custom.first-key"),
        ("second", "custom.second-key"),
        ("disabled", "custom.disabled-key"),
        ("deleted", "custom.deleted-key"),
        ("unavailable", "custom.unavailable-key"),
    ]
    .into_iter()
    .map(|(id, key)| {
        ClientPolicy::new(
            ClientApiKeyId::new(id).unwrap(),
            PlaintextClientApiKey::new(key).unwrap(),
            Arc::new(FrozenAccountScope::new(
                directory.clone(),
                ClientRoutingScope::all_accounts(),
            )),
            true,
            RateLimits::unlimited(),
        )
    })
    .collect();
    let snapshot = RuntimeSnapshot::new(
        ConfigRevision::new(1).unwrap(),
        AccountSelectionPolicy::new(
            RotationStrategy::Smart,
            NonZeroU32::new(2).unwrap(),
            Duration::from_millis(1),
        ),
        Vec::new(),
        Vec::new(),
        policies,
    )
    .unwrap()
    .with_min_codex_client_versions(CodexClientMinVersions::new(
        Some(CodexClientVersion::parse("99.0.0").unwrap()),
        Some(CodexClientVersion::parse("99.0.0").unwrap()),
    ));
    let handle = RuntimeSnapshotHandle::new(snapshot);
    if suspended {
        handle.suspend();
    }
    Arc::new(DefaultExecutionService::new(
        handle,
        Arc::new(UnusedExecutionStore),
        ProviderRegistry::default(),
        Arc::new(UnusedAdmissions),
        Arc::new(UnusedCircuits),
        Arc::new(UnusedContinuation),
        Arc::new(IgnoredClientApiKeyUsage),
    ))
}

#[tokio::test]
async fn usage_authenticates_only_bearer_and_keeps_budget_exhausted_keys_queryable() {
    let app = api_router(execution(false)).await;
    for (key, status, used, code) in [
        (
            None,
            StatusCode::UNAUTHORIZED,
            None,
            Some("invalid_api_key"),
        ),
        (
            Some("wrong-key"),
            StatusCode::UNAUTHORIZED,
            None,
            Some("invalid_api_key"),
        ),
        (Some("custom.first-key"), StatusCode::OK, Some("2"), None),
        (
            Some("custom.second-key"),
            StatusCode::OK,
            Some("0.25"),
            None,
        ),
        (
            Some("custom.disabled-key"),
            StatusCode::UNAUTHORIZED,
            None,
            Some("invalid_api_key"),
        ),
        (
            Some("custom.deleted-key"),
            StatusCode::UNAUTHORIZED,
            None,
            Some("invalid_api_key"),
        ),
        (
            Some("custom.unavailable-key"),
            StatusCode::SERVICE_UNAVAILABLE,
            None,
            Some("key_usage_unavailable"),
        ),
    ] {
        let mut request = Request::get("/v1/usage?id=second&key=custom.second-key")
            .header("cookie", "admin_session=example; key=custom.second-key")
            .header("user-agent", "codex_cli_rs");
        if let Some(key) = key {
            request = request.header("authorization", format!("Bearer {key}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        if let Some(code) = code {
            assert_eq!(body["error"]["code"], code);
        }
        if let Some(used) = used {
            assert_eq!(body["object"], "key_usage");
            assert_eq!(body["dailyUsedUsd"], used);
            assert_eq!(
                body["dailyRemainingUsd"],
                if used == "2" { "0" } else { "0.75" }
            );
            assert!(body["weeklyRemainingUsd"].is_null());
            assert!(body.get("key").is_none());
            assert!(body.get("id").is_none());
        }
    }
}

#[tokio::test]
async fn usage_reports_unavailable_snapshot_without_caching() {
    let app = api_router(execution(true)).await;
    let response = app
        .oneshot(
            Request::get("/v1/usage")
                .header("authorization", "Bearer custom.first-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "runtime_configuration_unavailable");
}
