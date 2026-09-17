use std::{
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use chrono::Utc;
use futures::future::BoxFuture;
use gateway_admin::ports::provider::ProviderAdminErrorKind;
use gateway_core::{
    account::{OutboundProxy, ProviderAccountId},
    lifecycle::CancellationToken,
    provider_ports::{ProviderRefreshPolicy, ProviderRuntimePolicyPort, ProviderStoreError},
    task::DaemonTask,
};
use provider_openai::{SESSION_KEEPALIVE_MODELS, SessionManager};
use provider_openai::{
    credential::ImportCodexOAuthCredential,
    transport::{
        profile::{CodexWireProfile, CodexWireProfileState},
        protocol::responses::CodexResponsesRequest,
    },
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, header, method},
};

use crate::support::{MemoryAccountStore, profile, secret};

struct Policy {
    proxy: Mutex<Option<OutboundProxy>>,
    reads: AtomicUsize,
}
impl ProviderRuntimePolicyPort for Policy {
    fn load_refresh_policy(
        &self,
    ) -> BoxFuture<'_, Result<ProviderRefreshPolicy, ProviderStoreError>> {
        Box::pin(async {
            ProviderRefreshPolicy::try_new(Duration::from_secs(60), NonZeroU32::new(2).unwrap())
        })
    }
    fn load_oam_proxy(&self) -> BoxFuture<'_, Result<Option<OutboundProxy>, ProviderStoreError>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let proxy = self.proxy.lock().unwrap().clone();
        Box::pin(async move { Ok(proxy) })
    }
}

async fn fixture(
    proxy: Option<&str>,
) -> (Arc<MemoryAccountStore>, Arc<Policy>, Arc<SessionManager>) {
    let store = Arc::new(MemoryAccountStore::default());
    for id in ["acct_a", "acct_b"] {
        store
            .seed_oauth_credential(ImportCodexOAuthCredential {
                account_id: id.to_owned(),
                name: id.to_owned(),
                secret: secret(id),
                verified_account: profile(id),
                next_refresh_at: None,
                enabled: true,
            })
            .await;
        store.set_session_keepalive(id, true);
    }
    let policy = Arc::new(Policy {
        proxy: Mutex::new(proxy.map(|url| OutboundProxy::parse(url).unwrap())),
        reads: AtomicUsize::new(0),
    });
    let profile = wire_profile();
    let manager = Arc::new(SessionManager::new(
        store.repository(),
        policy.clone(),
        profile,
        "http://upstream.invalid/backend-api".to_owned(),
    ));
    (store, policy, manager)
}

fn wire_profile() -> CodexWireProfileState {
    CodexWireProfileState::new(CodexWireProfile {
        originator: "codex_cli_rs".to_owned(),
        codex_version: "0.144.0".to_owned(),
        desktop_version: "1.0.0".to_owned(),
        desktop_build: "1".to_owned(),
        os_type: "linux".to_owned(),
        os_version: "6.8".to_owned(),
        arch: "x86_64".to_owned(),
        terminal: "session-test".to_owned(),
        residency: None,
        verified_at: Utc::now(),
    })
}

fn request(model: &str) -> CodexResponsesRequest {
    let mut request = CodexResponsesRequest::from_body(
        json!({"model":model,"input":[],"client_metadata":{"preserved":"value"}})
            .as_object()
            .unwrap()
            .clone(),
    );
    request.turn_state = Some("client-state".to_owned());
    request
}

fn success(state: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("x-codex-turn-state", state)
        .set_body_raw(
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n",
            "text/event-stream",
        )
}

async fn mock_model(proxy: &MockServer, account: &str, model: &str, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(header("authorization", format!("Bearer {account}")))
        .and(body_partial_json(
            json!({"model":model,"store":false,"stream":true}),
        ))
        .respond_with(response)
        .mount(proxy)
        .await;
}

#[tokio::test]
async fn refresh_and_rewrite_isolate_every_account_and_model_using_only_oam_proxy() {
    let oam = MockServer::start().await;
    let business = MockServer::start().await;
    let (store, _, manager) = fixture(Some(&oam.uri())).await;
    for account in ["acct_a", "acct_b"] {
        store.set_egress(
            account,
            Some(OutboundProxy::parse(&business.uri()).unwrap()),
            None,
        );
        for model in SESSION_KEEPALIVE_MODELS {
            mock_model(&oam, account, model, success(&format!("{account}:{model}"))).await;
        }
        let result = manager
            .refresh(&ProviderAccountId::new(account).unwrap())
            .await
            .unwrap();
        assert!(
            result.models.iter().all(|item| item.error.is_none()
                && item.expire_at.unwrap() > Utc::now().timestamp() + 3590)
        );
    }
    for account in ["acct_a", "acct_b"] {
        for model in SESSION_KEEPALIVE_MODELS {
            let mut request = request(model);
            manager
                .rewrite(&store.account(account).unwrap(), &mut request)
                .await;
            assert_eq!(
                request.turn_state.as_deref(),
                Some(format!("{account}:{model}").as_str())
            );
            assert_eq!(
                request.client_metadata().unwrap()["x-codex-turn-state"],
                format!("{account}:{model}")
            );
            assert_eq!(request.client_metadata().unwrap()["preserved"], "value");
        }
    }
    assert_eq!(oam.received_requests().await.unwrap().len(), 4);
    assert!(business.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn disabled_accounts_other_models_and_missing_proxy_never_refresh_or_override() {
    let proxy = MockServer::start().await;
    let (store, policy, manager) = fixture(Some(&proxy.uri())).await;
    store.set_session_keepalive("acct_a", false);
    let id = ProviderAccountId::new("acct_a").unwrap();
    assert_eq!(
        manager.refresh(&id).await.unwrap_err().kind(),
        ProviderAdminErrorKind::Invalid
    );
    let mut req = request("6");
    manager
        .rewrite(&store.account("acct_a").unwrap(), &mut req)
        .await;
    assert_eq!(req.turn_state.as_deref(), Some("client-state"));
    *policy.proxy.lock().unwrap() = None;
    assert!(
        manager
            .refresh(&ProviderAccountId::new("acct_b").unwrap())
            .await
            .is_err()
    );
    for model in ["gpt-6-astra", "6 ", "5.6 Sol"] {
        let mut req = request(model);
        manager
            .rewrite(&store.account("acct_b").unwrap(), &mut req)
            .await;
        assert_eq!(req.turn_state.as_deref(), Some("client-state"));
    }
    assert!(proxy.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn partial_failure_keeps_previous_state_without_extending_its_expiry() {
    let proxy = MockServer::start().await;
    let (store, _, manager) = fixture(Some(&proxy.uri())).await;
    for model in SESSION_KEEPALIVE_MODELS {
        mock_model(&proxy, "acct_a", model, success("old-state")).await;
    }
    let id = ProviderAccountId::new("acct_a").unwrap();
    manager.refresh(&id).await.unwrap();
    proxy.reset().await;
    mock_model(&proxy, "acct_a", "5.6 sol", ResponseTemplate::new(500)).await;
    mock_model(&proxy, "acct_a", "6", success("new-state")).await;
    let result = manager.refresh(&id).await.unwrap();
    assert!(result.models[0].error.is_some());
    assert!(result.models[1].error.is_none());
    let mut sol = request("5.6 sol");
    manager
        .rewrite(&store.account("acct_a").unwrap(), &mut sol)
        .await;
    assert_eq!(sol.turn_state.as_deref(), Some("old-state"));
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(3600)).await;
    let mut expired = request("5.6 sol");
    manager
        .rewrite(&store.account("acct_a").unwrap(), &mut expired)
        .await;
    assert_eq!(expired.turn_state.as_deref(), Some("client-state"));
}

#[tokio::test]
async fn duplicate_refresh_is_rejected_and_invalidated_inflight_results_cannot_repopulate() {
    let proxy = MockServer::start().await;
    let (store, _, manager) = fixture(Some(&proxy.uri())).await;
    for model in SESSION_KEEPALIVE_MODELS {
        mock_model(
            &proxy,
            "acct_a",
            model,
            success("stale-state").set_delay(Duration::from_millis(100)),
        )
        .await;
    }
    let id = ProviderAccountId::new("acct_a").unwrap();
    let pending = {
        let manager = Arc::clone(&manager);
        let id = id.clone();
        tokio::spawn(async move { manager.refresh(&id).await })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while proxy.received_requests().await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        manager.refresh(&id).await.unwrap_err().kind(),
        ProviderAdminErrorKind::Conflict
    );
    manager.invalidate(&id).await;
    let result = pending.await.unwrap().unwrap();
    assert!(result.models.iter().all(|item| item.error.is_some()));
    let mut req = request("6");
    manager
        .rewrite(&store.account("acct_a").unwrap(), &mut req)
        .await;
    assert_eq!(req.turn_state.as_deref(), Some("client-state"));
}

#[tokio::test]
async fn rate_limit_honors_retry_after_and_does_not_send_the_second_model() {
    let proxy = MockServer::start().await;
    let (_, _, manager) = fixture(Some(&proxy.uri())).await;
    mock_model(
        &proxy,
        "acct_a",
        "5.6 sol",
        ResponseTemplate::new(429).insert_header("retry-after", "60"),
    )
    .await;
    let id = ProviderAccountId::new("acct_a").unwrap();
    let first = manager.refresh(&id).await.unwrap();
    assert!(first.models.iter().all(|model| model.error.is_some()));
    let second = manager.refresh(&id).await.unwrap();
    assert!(second.models.iter().all(|model| model.error.is_some()));
    assert_eq!(proxy.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn missing_state_or_failed_sse_completion_never_creates_cache_entries() {
    let proxy = MockServer::start().await;
    let (store, _, manager) = fixture(Some(&proxy.uri())).await;
    mock_model(&proxy, "acct_a", "5.6 sol", ResponseTemplate::new(200)).await;
    mock_model(
        &proxy,
        "acct_a",
        "6",
        ResponseTemplate::new(200)
            .insert_header("x-codex-turn-state", "unusable")
            .set_body_raw(
                "data: {\"type\":\"response.failed\"}\n\n",
                "text/event-stream",
            ),
    )
    .await;
    let result = manager
        .refresh(&ProviderAccountId::new("acct_a").unwrap())
        .await
        .unwrap();
    assert!(result.models.iter().all(|model| model.error.is_some()));
    let mut req = request("6");
    manager
        .rewrite(&store.account("acct_a").unwrap(), &mut req)
        .await;
    assert_eq!(req.turn_state.as_deref(), Some("client-state"));
}

#[tokio::test]
async fn background_worker_stops_promptly_when_cancelled() {
    let (_, _, manager) = fixture(None).await;
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(1), manager.run(cancellation))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn rewritten_business_http_uses_original_proxy_and_never_oam_pool() {
    use futures::StreamExt;
    use provider_openai::transport::{CodexBackendClient, CodexRequestContext};
    let oam = MockServer::start().await;
    let business = MockServer::start().await;
    let (store, _, manager) = fixture(Some(&oam.uri())).await;
    store.set_egress(
        "acct_a",
        Some(OutboundProxy::parse(&business.uri()).unwrap()),
        None,
    );
    for model in SESSION_KEEPALIVE_MODELS {
        mock_model(&oam, "acct_a", model, success("rotated-state")).await;
    }
    manager
        .refresh(&ProviderAccountId::new("acct_a").unwrap())
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(header("x-codex-turn-state", "rotated-state"))
        .respond_with(success("business-state"))
        .expect(1)
        .mount(&business)
        .await;
    let account = store.account("acct_a").unwrap();
    let client = CodexBackendClient::new(
        reqwest::Client::builder().no_proxy().build().unwrap(),
        "http://upstream.invalid/backend-api",
        wire_profile(),
    )
    .for_account(&account)
    .unwrap();
    let mut req = request("6");
    req.force_http_sse = true;
    manager.rewrite(&account, &mut req).await;
    let context = CodexRequestContext {
        trace: None,
        authorization: "Bearer acct_a",
        account_id: Some("acct_a"),
        request_id: "business-request",
        turn_state: req.turn_state.as_deref(),
        turn_metadata: None,
        beta_features: None,
        include_timing_metrics: None,
        version: None,
        codex_window_id: None,
        parent_thread_id: None,
        cookie_header: None,
        installation_id: None,
        session_id: None,
        thread_id: None,
        client_request_id: None,
        turn_id: None,
        account_selection: Default::default(),
    };
    let mut response = client
        .create_response_stream_with_pool_account(&req, context, None)
        .await
        .unwrap();
    while let Some(chunk) = response.body.next().await {
        chunk.unwrap();
    }
    assert_eq!(business.received_requests().await.unwrap().len(), 1);
    assert_eq!(oam.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn credential_revision_change_prevents_reusing_previous_state() {
    use gateway_core::account::ProviderAccount;
    let proxy = MockServer::start().await;
    let (store, _, manager) = fixture(Some(&proxy.uri())).await;
    for model in SESSION_KEEPALIVE_MODELS {
        mock_model(&proxy, "acct_a", model, success("old-credential-state")).await;
    }
    let id = ProviderAccountId::new("acct_a").unwrap();
    manager.refresh(&id).await.unwrap();
    let current = store.account("acct_a").unwrap();
    let rotated = ProviderAccount::new(
        id,
        current.provider().clone(),
        current.name().to_owned(),
        current.upstream_user_id().map(str::to_owned),
        "oauth".to_owned(),
        current.revision().next().unwrap(),
        current.access_token_expires_at(),
    )
    .with_session_keepalive(true)
    .with_account_facts(
        true,
        current.credential_state(),
        current.quota(),
        None,
        None,
    );
    let mut req = request("6");
    manager.rewrite(&current, &mut req).await;
    assert_eq!(req.turn_state.as_deref(), Some("old-credential-state"));
    let mut req = request("6");
    manager.rewrite(&rotated, &mut req).await;
    assert_eq!(req.turn_state.as_deref(), Some("client-state"));
}

#[tokio::test(start_paused = true)]
async fn worker_waits_between_fifty_and_fifty_eight_minutes_and_cancels_sleep() {
    let (_, policy, manager) = fixture(None).await;
    let cancellation = CancellationToken::new();
    let task = {
        let cancellation = cancellation.clone();
        tokio::spawn(async move { manager.run(cancellation).await })
    };
    tokio::task::yield_now().await;
    assert_eq!(policy.reads.load(Ordering::SeqCst), 2);
    tokio::time::advance(Duration::from_secs(2999)).await;
    tokio::task::yield_now().await;
    assert_eq!(policy.reads.load(Ordering::SeqCst), 2);
    tokio::time::advance(Duration::from_secs(482)).await;
    tokio::task::yield_now().await;
    assert_eq!(policy.reads.load(Ordering::SeqCst), 4);
    cancellation.cancel();
    task.await.unwrap().unwrap();
}
