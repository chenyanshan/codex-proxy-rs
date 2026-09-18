mod diagnostics;
mod manager;

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
    matchers::{header, method},
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
    fn load_session_keepalive_proxy(
        &self,
    ) -> BoxFuture<'_, Result<Option<OutboundProxy>, ProviderStoreError>> {
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

// 合成凭证只用于验证严格长度合同。
fn state(label: &str) -> String {
    format!("{label:A<290}==")
}

fn success(label: &str) -> ResponseTemplate {
    response_with_state(&state(label))
}

fn response_with_state(state: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("x-codex-turn-state", state)
        .set_body_raw(
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n",
            "text/event-stream",
        )
}

async fn mock_model(proxy: &MockServer, account: &str, model: &str, response: ResponseTemplate) {
    let model = model.to_owned();
    Mock::given(method("POST"))
        .and(header("authorization", format!("Bearer {account}")))
        .and(move |request: &wiremock::Request| {
            let bytes = zstd::stream::decode_all(request.body.as_slice()).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            body["model"] == model
                && body["store"] == false
                && body["stream"] == true
                && body["input"][0]["content"][0]["text"] == "hi"
        })
        .respond_with(response)
        .mount(proxy)
        .await;
}

fn model_match(model: &'static str) -> impl wiremock::Match {
    move |request: &wiremock::Request| {
        let bytes = zstd::stream::decode_all(request.body.as_slice()).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        body["model"] == model
    }
}
