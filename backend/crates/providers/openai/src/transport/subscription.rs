//! 当前绑定账号的订阅周期安全投影，与令牌有效期和额度状态无关。

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{CodexBackendClient, CodexRequestContext, client::read_capped_response_body};

pub(crate) const SUBSCRIPTION_FIELD: &str = "subscription";
const MAX_SUBSCRIPTION_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexSubscription {
    pub account_id: String,
    pub expires_at: DateTime<Utc>,
    pub will_renew: Option<bool>,
    pub observed_at: DateTime<Utc>,
}

impl CodexBackendClient {
    /// 可选展示查询失败只返回未知，不改变凭据或额度状态，也不重试。
    pub async fn fetch_subscription(
        &self,
        context: CodexRequestContext<'_>,
        account_id: &str,
    ) -> Option<CodexSubscription> {
        if account_id.is_empty() || account_id.len() > 512 {
            return None;
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut url = reqwest::Url::parse(&self.base_url).ok()?;
            let base_path = url.path().trim_end_matches('/');
            let subscription_path = if base_path.ends_with("/backend-api") {
                format!("{base_path}/subscriptions")
            } else {
                "/backend-api/subscriptions".to_owned()
            };
            url.set_path(&subscription_path);
            url.set_query(None);
            url.set_fragment(None);
            url.query_pairs_mut().append_pair("account_id", account_id);
            let origin = url.origin().ascii_serialization();
            let headers = self.account_request_headers(context).ok()?;
            let response = self
                .client
                .get(url)
                .headers(headers)
                .header(reqwest::header::ACCEPT, "application/json")
                .header(reqwest::header::ORIGIN, &origin)
                .header(reqwest::header::REFERER, format!("{origin}/"))
                .send()
                .await
                .ok()?;
            if !response.status().is_success() {
                return None;
            }
            let body = read_capped_response_body(response, MAX_SUBSCRIPTION_BODY_BYTES)
                .await
                .ok()?;
            if body.limit_exceeded() {
                return None;
            }
            let value: serde_json::Value = serde_json::from_str(&body.into_string()).ok()?;
            let expires_at =
                DateTime::parse_from_rfc3339(value.get("active_until")?.as_str()?.trim())
                    .ok()?
                    .with_timezone(&Utc);
            Some(CodexSubscription {
                account_id: account_id.to_owned(),
                expires_at,
                will_renew: value.get("will_renew").and_then(serde_json::Value::as_bool),
                observed_at: Utc::now(),
            })
        })
        .await
        .ok()
        .flatten()
    }
}
