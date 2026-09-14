use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use gateway_api::admin;
use tower::ServiceExt as _;

use super::super::{AdminTestFixture, AdminTestState};

#[tokio::test]
async fn subscription_wire_uses_final_account_identity_for_list_read_and_refresh() {
    use gateway_admin::model::{
        Revision,
        accounts::{AccountPageItem, AccountRecord},
        provider_credentials::{ProviderQuota, ProviderSubscription},
    };
    use gateway_core::{
        account::{
            AccountStatusFacts, AccountWeight, CredentialState, QuotaState, resolve_account_status,
        },
        routing::ProviderKind,
    };
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-session");
    let now = chrono::Utc::now();
    let facts = AccountStatusFacts {
        enabled: true,
        credential_state: CredentialState::Ready,
        access_token_expires_at: None,
        quota: QuotaState::unknown(),
        rate_limited_until: None,
        last_error_reason: None,
        last_error_message: None,
    };
    *fixture.account.lock().unwrap() = Some(AccountPageItem {
        account: AccountRecord {
            id: "acct_subscription_wire".to_owned(),
            provider_kind: ProviderKind::new("openai").unwrap(),
            groups: vec![],
            name: "fixture".to_owned(),
            email: None,
            upstream_user_id: None,
            upstream_account_id: Some("account-b".to_owned()),
            plan_type: None,
            authentication_kind: "oauth".to_owned(),
            credential_revision: Revision::new(1).unwrap(),
            has_refresh_token: true,
            access_token_expires_at: None,
            next_refresh_at: None,
            enabled: true,
            concurrency_limit: None,
            weight: AccountWeight::default(),
            outbound_proxy: None,
            credential_state: CredentialState::Ready,
            credential_observed_at: now,
            quota: QuotaState::unknown(),
            last_error_reason: None,
            last_error_message: None,
            created_at: now,
            updated_at: now,
        },
        projection: resolve_account_status(&facts, std::time::SystemTime::now()),
    });
    for source in ["account-a", "account-b"] {
        *fixture.provider_quota.lock().unwrap() = Some(ProviderQuota {
            subscription: Some(ProviderSubscription {
                upstream_account_id: source.to_owned(),
                expires_at: now,
                will_renew: None,
                observed_at: now,
            }),
            plan_type: None,
            observed_at: Some(now),
            refresh_token_expires_at: None,
            windows: vec![],
            limit_reached: false,
            provider_data: None,
        });
        for (uri, post) in [
            ("/api/admin/accounts", false),
            (
                "/api/admin/accounts/quota?accountId=acct_subscription_wire",
                false,
            ),
            ("/api/admin/accounts/quota/refresh", true),
        ] {
            let response = admin::router::<AdminTestState>()
                .with_state(fixture.state())
                .oneshot(
                    Request::builder()
                        .method(if post { "POST" } else { "GET" })
                        .uri(uri)
                        .header("x-request-id", "req_subscription_wire")
                        .header(header::COOKIE, "cpr_admin_session=valid-session")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(if post {
                            Body::from(r#"{"accountId":"acct_subscription_wire"}"#)
                        } else {
                            Body::empty()
                        })
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let body: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            assert_eq!(status, StatusCode::OK, "{uri}: {body}");
            let account = if uri == "/api/admin/accounts" {
                &body["data"]["items"][0]
            } else {
                &body["data"]["account"]
            };
            for subscription in [&account["subscription"], &account["quota"]["subscription"]] {
                if source == "account-a" {
                    assert!(subscription.is_null(), "{uri}");
                } else {
                    assert_eq!(subscription.as_object().unwrap().len(), 3);
                    assert!(subscription["expiresAt"].is_string());
                    assert!(subscription["observedAt"].is_string());
                    assert!(subscription["willRenew"].is_null());
                    assert!(subscription.get("upstreamAccountId").is_none());
                }
            }
        }
    }
}

#[tokio::test]
async fn quota_forecast_requires_admin_and_valid_account_query() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-session");
    for (uri, authenticated, expected) in [
        (
            "/api/admin/accounts/quota-forecast?accountId=acct_test",
            false,
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/api/admin/accounts/quota-forecast",
            true,
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/admin/accounts/quota-forecast?accountId=bad",
            true,
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/admin/accounts/quota-forecast?accountId=acct_test&refresh=true",
            true,
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let mut request = Request::builder()
            .uri(uri)
            .header("x-request-id", "req_forecast");
        if authenticated {
            request = request.header(header::COOKIE, "cpr_admin_session=valid-session");
        }
        let response = admin::router::<AdminTestState>()
            .with_state(fixture.state())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{uri}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(value["data"].is_null());
        assert!(value["message"].is_string());
    }
}
