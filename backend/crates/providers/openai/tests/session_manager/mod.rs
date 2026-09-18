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
    rewrite: Mutex<gateway_core::provider_ports::SessionRewritePolicy>,
    reads: AtomicUsize,
    tickets: Arc<MemoryTickets>,
}
impl ProviderRuntimePolicyPort for Policy {
    fn load_session_rewrite_policy(
        &self,
    ) -> BoxFuture<'_, Result<gateway_core::provider_ports::SessionRewritePolicy, ProviderStoreError>>
    {
        let policy = *self.rewrite.lock().unwrap();
        Box::pin(async move { Ok(policy) })
    }

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
        tickets: Arc::new(MemoryTickets::default()),
        rewrite: Mutex::new(
            gateway_core::provider_ports::SessionRewritePolicy::try_new(1, 1).unwrap(),
        ),
    });
    let profile = wire_profile();
    let manager = Arc::new(SessionManager::new(
        store.repository(),
        policy.clone(),
        profile,
        "http://upstream.invalid/backend-api".to_owned(),
        Some(policy.tickets.clone()),
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
    format!("gAAAAA{label:A<284}==")
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

use gateway_core::provider_ports::{ProviderSessionTicket, ProviderSessionTicketPort};
#[derive(Default)]
pub(super) struct MemoryTickets {
    entries: Mutex<
        std::collections::HashMap<(String, String), (ProviderSessionTicket, tokio::time::Instant)>,
    >,
    unavailable: std::sync::atomic::AtomicBool,
}
impl ProviderSessionTicketPort for MemoryTickets {
    fn load<'a>(
        &'a self,
        account: &'a ProviderAccountId,
        model: &'a str,
    ) -> BoxFuture<'a, Result<Option<ProviderSessionTicket>, ProviderStoreError>> {
        Box::pin(async move {
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(ProviderStoreError::new(
                    gateway_core::provider_ports::ProviderStoreErrorKind::Unavailable,
                    "test cache",
                ));
            }
            Ok(self
                .entries
                .lock()
                .unwrap()
                .get(&(account.as_str().to_owned(), model.to_owned()))
                .filter(|(_, deadline)| *deadline > tokio::time::Instant::now())
                .map(|(ticket, _)| ticket.clone()))
        })
    }
    fn store<'a>(
        &'a self,
        account: &'a ProviderAccountId,
        model: &'a str,
        ticket: &'a ProviderSessionTicket,
    ) -> BoxFuture<'a, Result<(), ProviderStoreError>> {
        Box::pin(async move {
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(ProviderStoreError::new(
                    gateway_core::provider_ports::ProviderStoreErrorKind::Unavailable,
                    "test cache",
                ));
            }
            self.entries.lock().unwrap().insert(
                (account.as_str().to_owned(), model.to_owned()),
                (
                    ticket.clone(),
                    tokio::time::Instant::now()
                        + Duration::from_secs(
                            (ticket.expires_at - Utc::now().timestamp()).max(0) as u64
                        ),
                ),
            );
            Ok(())
        })
    }
    fn clear<'a>(
        &'a self,
        account: &'a ProviderAccountId,
    ) -> BoxFuture<'a, Result<(), ProviderStoreError>> {
        Box::pin(async move {
            self.entries
                .lock()
                .unwrap()
                .retain(|(id, _), _| id != account.as_str());
            Ok(())
        })
    }
}
impl MemoryTickets {
    fn near_expiry(&self) {
        for (ticket, _) in self.entries.lock().unwrap().values_mut() {
            ticket.expires_at = Utc::now().timestamp() + 599;
        }
    }
}

async fn capture_cookie(store: &Arc<MemoryAccountStore>) {
    use crate::support::{
        MemoryCooldownPort, MemorySessionAffinity, MemorySessionExclusions, TestLeaseCoordinator,
    };
    use gateway_core::{account::AccountFeedbackStats, routing::ProviderKind};
    use provider_openai::credential::{
        CodexCookiePolicy, CodexCredentialQuotaService, CodexCredentialSelector,
    };
    let leases = Arc::new(TestLeaseCoordinator::default());
    let quota = Arc::new(CodexCredentialQuotaService::new(
        store.repository(),
        wire_profile(),
        reqwest::Client::new(),
        provider_openai::OFFICIAL_CODEX_BASE_URL.to_owned(),
        Arc::new(MemoryCooldownPort::default()),
        leases.clone(),
        crate::support::runtime_policy(),
    ));
    let selector = CodexCredentialSelector::new(
        ProviderKind::new("openai").unwrap(),
        store.repository(),
        leases,
        Arc::new(MemorySessionAffinity::default()),
        Arc::new(MemorySessionExclusions::default()),
        quota,
        Arc::new(AccountFeedbackStats::default()),
        CodexCookiePolicy::official().unwrap(),
    );
    let outcome = selector
        .capture_response_cookies(
            &store.account("acct_a").unwrap(),
            &url::Url::parse("https://chatgpt.com/backend-api/codex/responses").unwrap(),
            &["__cf_bm=updated; Path=/; Domain=chatgpt.com; Secure; Max-Age=1800".to_owned()],
        )
        .await
        .unwrap();
    assert!(outcome.credential_revision.is_some());
}
