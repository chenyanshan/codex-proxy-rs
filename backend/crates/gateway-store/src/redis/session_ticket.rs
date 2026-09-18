//! State 票据使用绝对过期时间，重启和重复读取不会延长寿命。
use super::{namespace, resource_fingerprint};
use crate::StoreResult;
use futures::future::BoxFuture;
use gateway_core::{
    account::ProviderAccountId,
    provider_ports::{
        ProviderSessionTicket, ProviderSessionTicketPort, ProviderStoreError,
        ProviderStoreErrorKind,
    },
};
use redis::aio::ConnectionManager;

#[derive(Clone)]
pub struct RedisSessionTicketRepository {
    connection: ConnectionManager,
    namespace: String,
}
impl RedisSessionTicketRepository {
    pub fn new(connection: ConnectionManager, key_namespace: &str) -> StoreResult<Self> {
        Ok(Self {
            connection,
            namespace: namespace(key_namespace)?,
        })
    }
    fn key(&self, account: &ProviderAccountId) -> Result<String, ProviderStoreError> {
        let fingerprint =
            resource_fingerprint("session ticket", account.as_str()).map_err(|_| invalid())?;
        Ok(format!(
            "{}:session-ticket:{{{fingerprint}}}",
            self.namespace
        ))
    }
}
impl ProviderSessionTicketPort for RedisSessionTicketRepository {
    fn load<'a>(
        &'a self,
        account: &'a ProviderAccountId,
        model: &'a str,
    ) -> BoxFuture<'a, Result<Option<ProviderSessionTicket>, ProviderStoreError>> {
        Box::pin(async move {
            let raw: Option<String> = redis::cmd("HGET")
                .arg(self.key(account)?)
                .arg(model)
                .query_async(&mut self.connection.clone())
                .await
                .map_err(|_| unavailable())?;
            raw.map(|raw| serde_json::from_str(&raw).map_err(|_| invalid()))
                .transpose()
        })
    }
    fn store<'a>(
        &'a self,
        account: &'a ProviderAccountId,
        model: &'a str,
        ticket: &'a ProviderSessionTicket,
    ) -> BoxFuture<'a, Result<(), ProviderStoreError>> {
        Box::pin(async move {
            let now = chrono::Utc::now().timestamp();
            if ticket.expires_at <= now
                || ticket.expires_at > now + 3600
                || ticket.value.len() > 8192
            {
                return Err(invalid());
            }
            let payload = serde_json::to_string(ticket).map_err(|_| invalid())?;
            // 每个模型保留自身绝对期限；Hash 寿命只延长到其中最晚的期限。
            redis::Script::new(
                r#"
local now = tonumber(redis.call('TIME')[1])
if tonumber(ARGV[3]) <= now then return 0 end
redis.call('HSET', KEYS[1], ARGV[1], ARGV[2])
local expiry = redis.call('EXPIRETIME', KEYS[1])
if expiry < tonumber(ARGV[3]) then redis.call('EXPIREAT', KEYS[1], ARGV[3]) end
return 1
"#,
            )
            .key(self.key(account)?)
            .arg(model)
            .arg(payload)
            .arg(ticket.expires_at)
            .invoke_async::<i32>(&mut self.connection.clone())
            .await
            .map_err(|_| unavailable())
            .and_then(|saved| if saved == 1 { Ok(()) } else { Err(invalid()) })
        })
    }
    fn clear<'a>(
        &'a self,
        account: &'a ProviderAccountId,
    ) -> BoxFuture<'a, Result<(), ProviderStoreError>> {
        Box::pin(async move {
            redis::cmd("DEL")
                .arg(self.key(account)?)
                .query_async::<()>(&mut self.connection.clone())
                .await
                .map_err(|_| unavailable())
        })
    }
}
fn unavailable() -> ProviderStoreError {
    ProviderStoreError::new(ProviderStoreErrorKind::Unavailable, "session ticket cache")
}
fn invalid() -> ProviderStoreError {
    ProviderStoreError::new(ProviderStoreErrorKind::InvalidData, "session ticket cache")
}
