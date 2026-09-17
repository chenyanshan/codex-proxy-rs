//! 显式启用账号的实验性 State 轮换；运维网络与业务 transport 完全分离。

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::Utc;
use futures::{StreamExt, future::BoxFuture};
use gateway_admin::{
    model::accounts::{SessionModelRefresh, SessionStateRefresh},
    ports::provider::{ProviderAdminError, ProviderAdminErrorKind},
};
use gateway_core::{
    account::{
        CredentialRevision, CredentialState, OutboundProxy, ProviderAccount, ProviderAccountId,
    },
    lifecycle::CancellationToken,
    provider_ports::ProviderRuntimePolicyPort,
    task::{DaemonTask, WorkerTaskError},
};
use gateway_protocol::openai::sse::SseEventDecoder;
use reqwest::{Client, header::HeaderValue};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::{
    credential::CodexCredentialRepository,
    transport::{
        CODEX_RESPONSES_PATH, endpoint_url, headers::build_codex_model_headers,
        profile::CodexWireProfileState, protocol::responses::CodexResponsesRequest,
    },
};

pub const SESSION_KEEPALIVE_MODELS: [&str; 2] = ["5.6 sol", "6"];
const TTL_SECONDS: i64 = 3600;
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HEARTBEAT_BYTES: usize = 64 * 1024;

// State 不参与 Debug 或管理接口序列化，避免把路由凭证带出内存。
#[derive(Clone)]
struct SessionState {
    state_value: String,
    expire_at: i64,
}

struct CachedState {
    state: SessionState,
    credential_revision: CredentialRevision,
    deadline: tokio::time::Instant,
}

#[derive(Default)]
struct AccountSessions {
    refresh: Mutex<()>,
    states: RwLock<HashMap<String, CachedState>>,
    retry_after: Mutex<Option<tokio::time::Instant>>,
}

/// Provider 内自动与手动刷新共享的服务；也可用于显式装配 Provider。
pub struct SessionManager {
    repository: CodexCredentialRepository,
    policy: Arc<dyn ProviderRuntimePolicyPort>,
    profile: CodexWireProfileState,
    base_url: String,
    accounts: RwLock<HashMap<ProviderAccountId, Arc<AccountSessions>>>,
    oam_client: Mutex<Option<(OutboundProxy, Client)>>,
    capacity: Semaphore,
}

impl SessionManager {
    #[must_use]
    pub fn new(
        repository: CodexCredentialRepository,
        policy: Arc<dyn ProviderRuntimePolicyPort>,
        profile: CodexWireProfileState,
        base_url: String,
    ) -> Self {
        Self {
            repository,
            policy,
            profile,
            base_url,
            accounts: RwLock::new(HashMap::new()),
            oam_client: Mutex::new(None),
            capacity: Semaphore::new(2),
        }
    }

    /// 失效时移除当前代次；旧探活持有的 Arc 不能再写回新代次缓存。
    pub async fn invalidate(&self, account_id: &ProviderAccountId) {
        self.accounts.write().await.remove(account_id);
    }

    pub async fn refresh(
        &self,
        account_id: &ProviderAccountId,
    ) -> Result<SessionStateRefresh, ProviderAdminError> {
        let account = self
            .repository
            .store()
            .get_account(account_id)
            .await
            .map_err(|_| admin_error(ProviderAdminErrorKind::Unavailable, "账号读取失败"))?
            .ok_or_else(|| admin_error(ProviderAdminErrorKind::NotFound, "账号不存在"))?;
        if !eligible(&account) {
            return Err(admin_error(
                ProviderAdminErrorKind::Invalid,
                "账号未开启保活或凭据不可用",
            ));
        }
        let proxy = self
            .policy
            .load_oam_proxy()
            .await
            .map_err(|_| admin_error(ProviderAdminErrorKind::Unavailable, "运维代理配置读取失败"))?
            .ok_or_else(|| admin_error(ProviderAdminErrorKind::Invalid, "请先配置运维探活代理"))?;
        let sessions = self
            .accounts
            .write()
            .await
            .entry(account_id.clone())
            .or_default()
            .clone();
        let _refresh = sessions
            .refresh
            .try_lock()
            .map_err(|_| admin_error(ProviderAdminErrorKind::Conflict, "该账号正在刷新 State"))?;
        let _capacity = self.capacity.try_acquire().map_err(|_| {
            admin_error(ProviderAdminErrorKind::Conflict, "探活并发已满，请稍后重试")
        })?;
        let client = self.client(&proxy).await?;
        let credential = self
            .repository
            .load_runtime_credential(&account)
            .await
            .map_err(|_| admin_error(ProviderAdminErrorKind::Conflict, "账号凭据已变化，请重试"))?;
        let authorization = credential
            .authentication
            .authorization_header()
            .map_err(|_| admin_error(ProviderAdminErrorKind::Invalid, "账号鉴权不可用"))?;
        let mut models = Vec::with_capacity(SESSION_KEEPALIVE_MODELS.len());
        for model in SESSION_KEEPALIVE_MODELS {
            let result = if !account.model_access().allows(model) {
                Err("该账号未允许此模型")
            } else if sessions
                .retry_after
                .lock()
                .await
                .is_some_and(|deadline| deadline > tokio::time::Instant::now())
            {
                Err("上游要求稍后重试")
            } else {
                self.heartbeat(
                    &client,
                    &account,
                    authorization.expose_secret(),
                    &credential.installation_id,
                    model,
                    &sessions,
                )
                .await
            };
            let result = match result {
                Ok(state) => {
                    self.store_refreshed_state(&account, &proxy, &sessions, model, state)
                        .await
                }
                Err(error) => Err(error),
            };
            let (refreshed_at, expire_at, error) = match result {
                Ok(expire_at) => (Some(Utc::now()), Some(expire_at), None),
                Err(error) => (None, None, Some(error.to_owned())),
            };
            models.push(SessionModelRefresh {
                model: model.to_owned(),
                refreshed_at,
                expire_at,
                error,
            });
        }
        Ok(SessionStateRefresh {
            account_id: account_id.as_str().to_owned(),
            models,
        })
    }

    async fn client(&self, proxy: &OutboundProxy) -> Result<Client, ProviderAdminError> {
        let mut cached = self.oam_client.lock().await;
        if let Some((current, client)) = cached.as_ref()
            && current == proxy
        {
            return Ok(client.clone());
        }
        // 必须单独构造连接池；空代理在调用前已拒绝，绝不退回业务出口或环境代理。
        let client = Client::builder()
            .no_proxy()
            .proxy(
                reqwest::Proxy::all(proxy.expose_url())
                    .map_err(|_| admin_error(ProviderAdminErrorKind::Invalid, "运维代理无效"))?,
            )
            .danger_accept_invalid_certs(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(HEARTBEAT_TIMEOUT)
            .pool_max_idle_per_host(2)
            .build()
            .map_err(|_| {
                admin_error(ProviderAdminErrorKind::Unavailable, "运维客户端初始化失败")
            })?;
        *cached = Some((proxy.clone(), client.clone()));
        Ok(client)
    }

    async fn heartbeat(
        &self,
        client: &Client,
        account: &ProviderAccount,
        authorization: &str,
        installation_id: &str,
        model: &str,
        sessions: &AccountSessions,
    ) -> Result<String, &'static str> {
        let mut headers = build_codex_model_headers(
            &self.profile.snapshot(),
            authorization,
            account.upstream_account_id(),
        )
        .map_err(|_| "探活请求头无效")?;
        headers.insert(
            "x-codex-installation-id",
            HeaderValue::from_str(installation_id).map_err(|_| "账号身份无效")?,
        );
        headers.insert("accept", HeaderValue::from_static("text/event-stream"));
        let response = client.post(endpoint_url(&self.base_url, CODEX_RESPONSES_PATH))
            .headers(headers)
            .json(&json!({"model": model, "instructions": "Reply with one word.", "input": [{"role":"user", "content":[{"type":"input_text", "text":"hi"}]}], "tools": [], "store": false, "stream": true}))
            .send().await.map_err(|_| "运维网络请求失败或超时")?;
        if response.status().as_u16() == 429 {
            let delay =
                crate::transport::retry_after_seconds(response.headers(), None).unwrap_or(60);
            *sessions.retry_after.lock().await = tokio::time::Instant::now()
                .checked_add(Duration::from_secs(delay.min(u64::from(u32::MAX))));
        }
        if !response.status().is_success() {
            return Err("上游拒绝探活请求");
        }
        let state = crate::transport::turn_state(response.headers())
            .filter(|value| !value.is_empty() && value.len() <= 8192)
            .ok_or("上游未返回有效 State")?;
        let mut bytes = 0usize;
        let mut decoder = SseEventDecoder::default();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "探活响应中断或超时")?;
            bytes = bytes.saturating_add(chunk.len());
            if bytes > MAX_HEARTBEAT_BYTES {
                return Err("探活响应超出上限");
            }
            for event in decoder.push(&chunk).map_err(|_| "探活响应格式无效")? {
                let Ok(value) = serde_json::from_str::<Value>(&event.data) else {
                    continue;
                };
                match value.get("type").and_then(Value::as_str) {
                    Some("response.completed") => {
                        // 只记录数值用量；未知用量不伪造为零，不输出完整上游响应。
                        tracing::info!(
                            model,
                            input_tokens = value
                                .pointer("/response/usage/input_tokens")
                                .and_then(serde_json::Value::as_u64),
                            output_tokens = value
                                .pointer("/response/usage/output_tokens")
                                .and_then(serde_json::Value::as_u64),
                            "Session heartbeat completed"
                        );
                        return Ok(state);
                    }
                    Some("response.failed" | "response.incomplete" | "error") => {
                        return Err("上游探活未成功完成");
                    }
                    _ => {}
                }
            }
        }
        Err("探活响应未完整结束")
    }

    async fn store_refreshed_state(
        &self,
        account: &ProviderAccount,
        proxy: &OutboundProxy,
        sessions: &Arc<AccountSessions>,
        model: &str,
        state: String,
    ) -> Result<i64, &'static str> {
        let current = self
            .repository
            .store()
            .get_account(account.id())
            .await
            .map_err(|_| "账号状态校验失败")?;
        if !current.as_ref().is_some_and(|current| {
            eligible(current)
                && current.revision() == account.revision()
                && current.model_access().allows(model)
        }) {
            return Err("账号状态已变化，已丢弃探活结果");
        }
        let current_proxy = self
            .policy
            .load_oam_proxy()
            .await
            .map_err(|_| "运维配置校验失败")?;
        if current_proxy.as_ref() != Some(proxy) {
            return Err("运维代理已变化，已丢弃探活结果");
        }
        let accounts = self.accounts.read().await;
        if !accounts
            .get(account.id())
            .is_some_and(|current| Arc::ptr_eq(current, sessions))
        {
            return Err("账号配置已变化，已丢弃探活结果");
        }
        let expire_at = Utc::now().timestamp() + TTL_SECONDS;
        sessions.states.write().await.insert(
            cache_key(account.id(), model),
            CachedState {
                state: SessionState {
                    state_value: state,
                    expire_at,
                },
                credential_revision: account.revision(),
                deadline: tokio::time::Instant::now() + Duration::from_secs(TTL_SECONDS as u64),
            },
        );
        Ok(expire_at)
    }

    /// 仅改写已选号请求的 State；不接收或替换业务 Client。
    pub async fn rewrite(&self, account: &ProviderAccount, request: &mut CodexResponsesRequest) {
        if !eligible(account) || !SESSION_KEEPALIVE_MODELS.contains(&request.model()) {
            return;
        }
        let Some(sessions) = self.accounts.read().await.get(account.id()).cloned() else {
            return;
        };
        let states = sessions.states.read().await;
        let Some(cached) = states.get(&cache_key(account.id(), request.model())) else {
            return;
        };
        if cached.credential_revision != account.revision()
            || cached.state.expire_at <= Utc::now().timestamp()
            || cached.deadline <= tokio::time::Instant::now()
        {
            return;
        }
        let state = cached.state.state_value.clone();
        drop(states);
        // HTTP 头和复用 WS 连接的逐帧 metadata 使用同一值。
        request.passthrough_headers.remove("x-codex-turn-state");
        request.turn_state = Some(state.clone());
        let mut metadata = request
            .client_metadata()
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        metadata.insert("x-codex-turn-state".to_owned(), Value::String(state));
        request.set_client_metadata(Some(Value::Object(metadata)));
    }

    async fn refresh_cycle(&self) {
        let Ok(accounts) = self.repository.list_for_provider().await else {
            tracing::warn!("Session keepalive account list unavailable");
            return;
        };
        self.accounts.write().await.retain(|id, _| {
            accounts
                .iter()
                .any(|account| account.id() == id && eligible(account))
        });
        for account in accounts.iter().filter(|account| eligible(account)) {
            match self.refresh(account.id()).await {
                Ok(result) => {
                    for model in result.models {
                        if let Some(error) = model.error {
                            tracing::warn!(
                                account_id = account.id().as_str(),
                                model = model.model,
                                error,
                                "Session keepalive model refresh failed"
                            );
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(kind = ?error.kind(), "Session keepalive refresh unavailable")
                }
            }
        }
    }
}

impl DaemonTask for SessionManager {
    fn run(&self, cancellation: CancellationToken) -> BoxFuture<'_, Result<(), WorkerTaskError>> {
        Box::pin(async move {
            loop {
                tokio::select! {
                    () = cancellation.cancelled() => return Ok(()),
                    () = self.refresh_cycle() => {}
                }
                // 拒绝采样保证闭区间内等概率，失败交给 Host 监督而不是忙等。
                let seconds = loop {
                    let mut bytes = [0u8; 4];
                    getrandom::fill(&mut bytes)
                        .map_err(|_| WorkerTaskError::safe("session jitter unavailable"))?;
                    let sample = u32::from_le_bytes(bytes);
                    if sample < u32::MAX - u32::MAX % 481 {
                        break 3000 + u64::from(sample % 481);
                    }
                };
                tokio::select! {
                    () = cancellation.cancelled() => return Ok(()),
                    () = tokio::time::sleep(Duration::from_secs(seconds)) => {}
                }
            }
        })
    }
}

fn eligible(account: &ProviderAccount) -> bool {
    account.provider().as_str() == "openai"
        && account.authentication_kind() == "oauth"
        && account.enabled()
        && account.enable_session_keepalive()
        && account.credential_state() == CredentialState::Ready
        && account
            .access_token_expires_at()
            .is_none_or(|expires| expires > std::time::SystemTime::now())
}

fn cache_key(account_id: &ProviderAccountId, model: &str) -> String {
    format!("{}:{model}", account_id.as_str())
}

fn admin_error(kind: ProviderAdminErrorKind, message: &'static str) -> ProviderAdminError {
    ProviderAdminError::new(kind).with_public_message(message)
}
