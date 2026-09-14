//! Client Key 自助用量查询，不进入模型请求准入或执行。

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use gateway_core::metering::Decimal;
use serde_json::json;

use crate::ApiState;

use super::{
    auth::{bearer_client_api_key, client_access_error_response},
    error::{missing_client_api_key_response, openai_error_response},
};

pub(crate) async fn usage(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    let mut response = query_usage(&state, &headers).await;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn query_usage(state: &ApiState, headers: &HeaderMap) -> Response {
    // 用量查询沿用 Key 认证活动记录，但不要求 Codex 版本，也不消耗预算或并发。
    let client =
        match bearer_client_api_key(headers).and_then(|key| state.openai().authenticate(key)) {
            Ok(client) => client,
            Err(error) => return client_access_error_response(error.into()),
        };
    let usage = match state
        .admin
        .client_keys()
        .usage(client.policy().key_id())
        .await
    {
        Ok(Some(usage)) => usage,
        Ok(None) => return missing_client_api_key_response().into_response(),
        Err(_) => {
            return openai_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Key usage is temporarily unavailable",
                "server_error",
                "key_usage_unavailable",
            )
            .into_response();
        }
    };
    let budget = usage.budget;
    Json(json!({
        "object": "key_usage",
        "asOf": usage.as_of,
        "currency": "USD",
        "timezone": "Asia/Shanghai",
        "dailyLimitUsd": budget.limits.daily_usd.canonical(),
        "dailyUsedUsd": budget.daily_used_usd.canonical(),
        "dailyRemainingUsd": remaining(budget.limits.daily_usd, budget.daily_used_usd),
        "dailyResetsAt": budget.daily_resets_at.map(DateTime::<Utc>::from),
        "weeklyLimitUsd": budget.limits.weekly_usd.canonical(),
        "weeklyUsedUsd": budget.weekly_used_usd.canonical(),
        "weeklyRemainingUsd": remaining(budget.limits.weekly_usd, budget.weekly_used_usd),
        "weeklyResetsAt": budget.weekly_resets_at.map(DateTime::<Utc>::from),
        "maxConcurrency": usage.limits.max_concurrency,
        "requestsPerMinute": usage.limits.requests_per_minute,
    }))
    .into_response()
}

fn remaining(limit: Decimal, used: Decimal) -> Option<String> {
    if limit == Decimal::ZERO {
        return None;
    }
    // 相减后的非负缩放值不可能大于已验证的限额。
    Decimal::from_scaled(limit.scaled().saturating_sub(used.scaled()))
        .ok()
        .map(Decimal::canonical)
}
