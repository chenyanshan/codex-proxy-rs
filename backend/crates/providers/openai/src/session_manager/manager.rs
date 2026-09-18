//! 显式启用账号的实验性 State 轮换；运维网络与业务 transport 完全分离。

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::Utc;
use futures::{
    StreamExt,
    future::{BoxFuture, join_all, select_ok},
};
use gateway_admin::{
    model::accounts::{SessionModelRefresh, SessionRefreshObserver, SessionStateRefresh},
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
use reqwest::Client;

use super::diagnostics;
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::{
    credential::CodexCredentialRepository,
    transport::{
        CodexBackendClient, CodexRequestContext,
        profile::CodexWireProfileState,
        protocol::responses::CodexResponsesRequest,
        request::{RequestAccountScope, scope_request_to_account},
    },
};

pub const SESSION_KEEPALIVE_MODELS: [&str; 2] = ["gpt-5.6-sol", "gpt-6-astra"];
const TTL_SECONDS: i64 = 3600;
const TURN_STATE_LENGTH: usize = 292;
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
struct SessionCache {
    generation: u64,
    cancelled: tokio_util::sync::CancellationToken,
    states: HashMap<String, CachedState>,
}

#[derive(Default)]
struct AccountSessions {
    refresh: Mutex<()>,
    cache: RwLock<SessionCache>,
    retry_after: Mutex<HashMap<String, tokio::time::Instant>>,
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

    /// 只失效缓存代次，保留账号级刷新互斥与上游冷却，避免配置编辑绕过它们。
    pub async fn invalidate(&self, account_id: &ProviderAccountId) {
        let accounts = self.accounts.read().await;
        if let Some(sessions) = accounts.get(account_id) {
            let mut cache = sessions.cache.write().await;
            cache.generation = cache.generation.wrapping_add(1);
            cache.states.clear();
            cache.cancelled.cancel();
            cache.cancelled = tokio_util::sync::CancellationToken::new();
        }
    }

    pub async fn refresh(
        &self,
        account_id: &ProviderAccountId,
    ) -> Result<SessionStateRefresh, ProviderAdminError> {
        self.refresh_with_progress(account_id, None).await
    }

    pub async fn refresh_with_progress(
        &self,
        account_id: &ProviderAccountId,
        observer: Option<SessionRefreshObserver>,
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
            .load_session_keepalive_proxy()
            .await
            .map_err(|_| admin_error(ProviderAdminErrorKind::Unavailable, "动态代理配置读取失败"))?
            .ok_or_else(|| {
                admin_error(
                    ProviderAdminErrorKind::Invalid,
                    "请开启全局会话保活，并在代理管理中保存、测试动态代理",
                )
            })?;
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
        sessions
            .retry_after
            .lock()
            .await
            .retain(|_, deadline| *deadline > tokio::time::Instant::now());
        let (generation, cancelled) = {
            let cache = sessions.cache.read().await;
            (cache.generation, cache.cancelled.clone())
        };
        let _capacity = self.capacity.try_acquire().map_err(|_| {
            admin_error(ProviderAdminErrorKind::Conflict, "重写并发已满，请稍后重试")
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
        gateway_core::account::validate_session_keepalive_models(
            account.session_keepalive_models(),
        )
        .map_err(|_| admin_error(ProviderAdminErrorKind::Invalid, "请配置有效的重写模型"))?;
        // 每个模型拥有独立的重试 future；成功即写缓存，不等待其他模型。
        // join_all 持有完整 future 树，调用方取消时不会留下脱离生命周期的任务。
        let models = join_all(account.session_keepalive_models().iter().map(|model| {
            let account = &account;
            let proxy = &proxy;
            let sessions = &sessions;
            let client = &client;
            let authorization = &authorization;
            let installation_id = &credential.installation_id;
            let observer = &observer;
            let cancelled = &cancelled;
            async move {
                let result = tokio::select! {
                    biased;
                    () = cancelled.cancelled() => Err("账号配置已变化，已停止重写".to_owned()),
                    result = async {
                        let state = self.refresh_model(
                            client, account, proxy, authorization.expose_secret(),
                            installation_id, model, sessions, generation,
                        ).await?;
                        self.store_refreshed_state(account, proxy, sessions, generation, model, state).await.map_err(str::to_owned)
                    } => result,
                };
                let (refreshed_at, expire_at, error) = match result {
                    Ok(expire_at) => (Some(Utc::now()), Some(expire_at), None),
                    Err(error) => (None, None, Some(error)),
                };
                tracing::info!(target: "session_keepalive", account_id = account_id.as_str(), model, expire_at, error = error.as_deref(), "Session state cache update result");
                let result = SessionModelRefresh {
                    model: model.to_string(),
                    refreshed_at,
                    expire_at,
                    error,
                };
                if let Some(observer) = observer {
                    observer(result.clone());
                }
                result
            }
        }))
        .await;
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

    #[allow(clippy::too_many_arguments)]
    async fn refresh_model(
        &self,
        client: &Client,
        account: &ProviderAccount,
        proxy: &OutboundProxy,
        authorization: &str,
        installation_id: &str,
        model: &str,
        sessions: &AccountSessions,
        generation: u64,
    ) -> Result<String, String> {
        let mut attempt = 1u32;
        loop {
            // 无限重试仍须遵守配置变更，不能持续使用旧账号或旧代理。
            self.validate_refresh_context(account, proxy, sessions, generation, model)
                .await?;
            // 冷却属于账号＋模型，取消或配置失效后的新刷新也不能绕过。
            let cooldown = sessions.retry_after.lock().await.get(model).copied();
            if let Some(deadline) = cooldown
                && deadline > tokio::time::Instant::now()
            {
                tokio::time::sleep_until(deadline).await;
                continue;
            }
            let policy = self
                .policy
                .load_session_rewrite_policy()
                .await
                .map_err(|_| "State 重写参数读取失败".to_owned())?;
            let concurrency = policy.concurrency();
            let probes = (0..concurrency).map(|_| {
                Box::pin(self.heartbeat(
                    client,
                    (account, proxy, attempt),
                    authorization,
                    installation_id,
                    model,
                    &sessions.retry_after,
                ))
            });
            match select_ok(probes).await {
                Ok((state, remaining)) => {
                    // 未 spawn 的 HTTP future 在 drop 时立即取消，含正在读取的 SSE。
                    drop(remaining);
                    return Ok(state);
                }
                Err(error) => {
                    tracing::warn!(
                        account_id = account.id().as_str(),
                        model,
                        concurrency,
                        error,
                        "Session heartbeat round failed"
                    );
                }
            }
            let mut deadline = tokio::time::Instant::now()
                + Duration::from_secs(u64::from(policy.retry_interval_seconds()));
            // 限流只延长当前模型的等待；并发响应不能缩短已收到的 Retry-After。
            if let Some(upstream_deadline) = sessions.retry_after.lock().await.get(model).copied() {
                deadline = deadline.max(upstream_deadline);
            }
            tokio::time::sleep_until(deadline).await;
            attempt = attempt.saturating_add(1);
        }
    }

    async fn heartbeat(
        &self,
        client: &Client,
        target: (&ProviderAccount, &OutboundProxy, u32),
        authorization: &str,
        installation_id: &str,
        model: &str,
        retry_after: &Mutex<HashMap<String, tokio::time::Instant>>,
    ) -> Result<String, String> {
        let (account, proxy, attempt) = target;
        let probe_id = uuid::Uuid::new_v4().to_string();
        let proxy_url = reqwest::Url::parse(proxy.expose_url()).map_err(|_| "运维代理无效")?;
        let log = diagnostics::ProbeLog {
            account_id: account.id().as_str(),
            model,
            probe_id: &probe_id,
            attempt,
            secrets: vec![
                authorization,
                authorization
                    .strip_prefix("Bearer ")
                    .unwrap_or(authorization),
                proxy.expose_url(),
                proxy_url.password().unwrap_or(""),
            ],
        };
        let started = std::time::Instant::now();
        let operation = crate::transport::request::build_connection_test_operation(
            &gateway_core::routing::UpstreamModelId::new(model).map_err(|_| "模型 ID 无效")?,
            "hi",
        )
        .map_err(|_| "账户测试请求构造失败")?;
        let gateway_core::operation::Operation::Generate(generate) = operation else {
            return Err("账户测试请求类型无效".to_owned());
        };
        let mut request = crate::encode_generate_request(&generate, model, None)
            .map_err(|_| "账户测试请求编码失败")?;
        scope_request_to_account(&mut request, installation_id, RequestAccountScope::Unknown);
        let backend = CodexBackendClient::new(client.clone(), &self.base_url, self.profile.clone());
        let context = CodexRequestContext {
            trace: None,
            authorization,
            account_id: account.upstream_account_id(),
            request_id: &probe_id,
            turn_state: None,
            turn_metadata: None,
            beta_features: None,
            include_timing_metrics: None,
            version: None,
            codex_window_id: None,
            parent_thread_id: None,
            cookie_header: None,
            installation_id: Some(installation_id),
            session_id: None,
            thread_id: None,
            client_request_id: None,
            turn_id: None,
            account_selection: Default::default(),
        };
        let headers = backend
            .request_headers_for_http_response(&request, context)
            .map_err(|_| "重写请求头无效")?;
        let body = serde_json::to_vec(request.body()).map_err(|_| "重写请求编码失败")?;
        let outgoing = backend
            .build_http_sse_request(headers, body)
            .map_err(|_| "重写请求编码失败")?;
        log.record("request", json!({"method":outgoing.method().as_str(), "path":outgoing.url().path(), "proxyEndpoint":proxy.endpoint(), "headers":diagnostics::headers(outgoing.headers()), "body":request.body()}));
        let response = client.execute(outgoing).await.map_err(|error| {
            log.record("transport_error", json!({"timeout":error.is_timeout(), "connect":error.is_connect(), "error":error.without_url().to_string(), "elapsedMs":started.elapsed().as_millis()}));
            format!("运维网络请求失败或超时（重写 {probe_id}）")
        })?;
        let status = response.status();
        if status.as_u16() == 429 {
            let delay =
                crate::transport::retry_after_seconds(response.headers(), None).unwrap_or(60);
            let deadline =
                tokio::time::Instant::now() + Duration::from_secs(delay.min(u64::from(u32::MAX)));
            let mut current = retry_after.lock().await;
            current
                .entry(model.to_owned())
                .and_modify(|previous| *previous = (*previous).max(deadline))
                .or_insert(deadline);
        }
        // 先断言 Header 原始字节长度，不复制、不 trim、不等待响应正文。
        if let Some(state) = response.headers().get("x-codex-turn-state")
            && state.as_bytes().len() != TURN_STATE_LENGTH
        {
            log.record("invalid_state", json!({"status":status.as_u16(), "stateLength":state.as_bytes().len(), "elapsedMs":started.elapsed().as_millis()}));
            return Err(format!("上游 State 长度无效（重写 {probe_id}）"));
        }
        log.record("response_headers", json!({"status":status.as_u16(), "headers":diagnostics::headers(response.headers()), "elapsedMs":started.elapsed().as_millis()}));
        let state = crate::transport::turn_state(response.headers());
        if status.is_success() && state.is_none() {
            return Err(format!("上游未返回有效 State（重写 {probe_id}）"));
        }
        let mut bytes = Vec::new();
        let mut decoder = SseEventDecoder::default();
        let mut stream = response.bytes_stream();
        let mut completion = None;
        let mut read_error = None;
        let mut truncated = false;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(_) => {
                    read_error = Some("重写响应中断或超时");
                    break;
                }
            };
            let remaining = MAX_HEARTBEAT_BYTES.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            if chunk.len() > remaining {
                truncated = true;
                break;
            }
            if status.is_success() {
                let events = match decoder.push(&chunk) {
                    Ok(events) => events,
                    Err(_) => {
                        read_error = Some("重写响应格式无效");
                        break;
                    }
                };
                for event in events {
                    if let Ok(value) = serde_json::from_str::<Value>(&event.data) {
                        match value.get("type").and_then(Value::as_str) {
                            Some("response.completed") => completion = Some(true),
                            Some("response.failed" | "response.incomplete" | "error") => {
                                completion = Some(false)
                            }
                            _ => {}
                        }
                    }
                }
                if completion.is_some() {
                    break;
                }
            }
        }
        let content = String::from_utf8_lossy(&bytes);
        let parsed = diagnostics::response_body(&bytes, truncated || read_error.is_some());
        log.record("response_body", json!({"status":status.as_u16(), "body":parsed, "truncated":truncated, "readError":read_error, "state":state, "elapsedMs":started.elapsed().as_millis()}));
        if !status.is_success() {
            let mut message = serde_json::from_str::<Value>(&content)
                .ok()
                .and_then(|body| {
                    body.pointer("/error/message")
                        .or_else(|| body.get("message"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "上游拒绝重写请求，详情见重写日志".to_owned());
            let mut safe = Value::String(message);
            diagnostics::redact(&mut safe, &log.secrets);
            message = safe
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(512)
                .collect();
            return Err(format!(
                "HTTP {}：{}（重写 {}）",
                status.as_u16(),
                message,
                probe_id
            ));
        }
        if truncated {
            return Err(format!("重写响应超出上限（重写 {probe_id}）"));
        }
        if let Some(error) = read_error {
            return Err(format!("{error}（重写 {probe_id}）"));
        }
        if completion != Some(true) {
            return Err(format!("上游重写未成功完成（重写 {probe_id}）"));
        }
        state.ok_or_else(|| format!("上游未返回有效 State（重写 {probe_id}）"))
    }

    async fn validate_refresh_context(
        &self,
        account: &ProviderAccount,
        proxy: &OutboundProxy,
        sessions: &AccountSessions,
        generation: u64,
        model: &str,
    ) -> Result<(), &'static str> {
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
                && current
                    .session_keepalive_models()
                    .iter()
                    .any(|selected| selected == model)
        }) {
            return Err("账号状态已变化，已丢弃重写结果");
        }
        let current_proxy = self
            .policy
            .load_session_keepalive_proxy()
            .await
            .map_err(|_| "运维配置校验失败")?;
        if current_proxy.as_ref() != Some(proxy) {
            return Err("运维代理已变化，已丢弃重写结果");
        }
        if sessions.cache.read().await.generation != generation {
            return Err("账号配置已变化，已停止探活");
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn store_refreshed_state(
        &self,
        account: &ProviderAccount,
        proxy: &OutboundProxy,
        sessions: &AccountSessions,
        generation: u64,
        model: &str,
        state: String,
    ) -> Result<i64, &'static str> {
        self.validate_refresh_context(account, proxy, sessions, generation, model)
            .await?;
        let mut cache = sessions.cache.write().await;
        if cache.generation != generation {
            return Err("账号配置已变化，已丢弃重写结果");
        }
        let expire_at = Utc::now().timestamp() + TTL_SECONDS;
        cache.states.insert(
            model.to_owned(),
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
        if !eligible(account)
            || !account
                .session_keepalive_models()
                .iter()
                .any(|model| model == request.model())
        {
            return;
        }
        let Some(sessions) = self.accounts.read().await.get(account.id()).cloned() else {
            return;
        };
        let cache = sessions.cache.read().await;
        let Some(cached) = cache.states.get(request.model()) else {
            return;
        };
        if cached.credential_revision != account.revision()
            || cached.state.expire_at <= Utc::now().timestamp()
            || cached.deadline <= tokio::time::Instant::now()
        {
            return;
        }
        let state = cached.state.state_value.clone();
        drop(cache);
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
        // 总开关关闭时连账号列表也不遍历；手动刷新仍通过同一持久策略检查。
        if !matches!(
            self.policy.load_session_keepalive_proxy().await,
            Ok(Some(_))
        ) {
            return;
        }
        let Ok(accounts) = self.repository.list_for_provider().await else {
            tracing::warn!("Session keepalive account list unavailable");
            return;
        };
        self.accounts.write().await.retain(|id, sessions| {
            accounts
                .iter()
                .any(|account| account.id() == id && eligible(account))
                // 仍被请求持有或处于冷却期的账号不能换锁；重新启用也须遵守原有边界。
                || Arc::strong_count(sessions) > 1
                || sessions.retry_after.try_lock().map_or(true, |retry_after| {
                    retry_after.values().any(|deadline| *deadline > tokio::time::Instant::now())
                })
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
                    if sample < u32::MAX - u32::MAX % 121 {
                        break 3180 + u64::from(sample % 121);
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

fn admin_error(kind: ProviderAdminErrorKind, message: &'static str) -> ProviderAdminError {
    ProviderAdminError::new(kind).with_public_message(message)
}
