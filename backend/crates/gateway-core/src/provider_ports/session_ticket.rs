//! 可恢复的短期 Provider 会话票据；具体准入规则由 Provider 解释。
use super::ProviderStoreError;
use crate::account::ProviderAccountId;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

// 不实现 Debug，票据原文只能用于存储与对应账号的出站请求。
#[derive(Clone, Serialize, Deserialize)]
pub struct ProviderSessionTicket {
    pub value: String,
    pub credential_revision: u64,
    /// Provider 定义的鉴权绑定；旧缓存没有此字段时仍按原版本校验。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_binding: Option<[u8; 32]>,
    pub expires_at: i64,
}

pub trait ProviderSessionTicketPort: Send + Sync {
    fn load<'a>(
        &'a self,
        account: &'a ProviderAccountId,
        model: &'a str,
    ) -> BoxFuture<'a, Result<Option<ProviderSessionTicket>, ProviderStoreError>>;
    fn store<'a>(
        &'a self,
        account: &'a ProviderAccountId,
        model: &'a str,
        ticket: &'a ProviderSessionTicket,
    ) -> BoxFuture<'a, Result<(), ProviderStoreError>>;
    fn clear<'a>(
        &'a self,
        account: &'a ProviderAccountId,
    ) -> BoxFuture<'a, Result<(), ProviderStoreError>>;
}
