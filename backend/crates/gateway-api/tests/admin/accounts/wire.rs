use chrono::{TimeZone as _, Utc};
use gateway_admin::model::{
    AdminError,
    provider_credentials::{AccountPersonalInfo, ProviderSubscription},
};
use gateway_api::admin::accounts::{AccountPersonalInfoData, AccountSubscriptionData};
use serde_json::json;

#[test]
fn subscription_response_exposes_only_display_fields_and_preserves_unknowns() {
    let observed = Utc.with_ymd_and_hms(2026, 9, 14, 0, 0, 0).unwrap();
    let response = AccountSubscriptionData::from(ProviderSubscription {
        starts_at: None,
        expires_at: observed,
        will_renew: None,
        billing_period: None,
        billing_currency: Some("USD".to_owned()),
        observed_at: observed,
    });
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "startsAt": null, "expiresAt": "2026-09-14T00:00:00+00:00",
            "willRenew": null, "billingPeriod": null, "billingCurrency": "USD",
            "observedAt": "2026-09-14T00:00:00+00:00"
        })
    );
}

#[test]
fn personal_info_response_preserves_subscription_when_profile_fails() {
    let observed = Utc.with_ymd_and_hms(2026, 9, 14, 0, 0, 0).unwrap();
    let response = AccountPersonalInfoData::from(AccountPersonalInfo {
        profile: Err(AdminError::bad_gateway("上游服务请求失败")),
        subscription: Some(ProviderSubscription {
            starts_at: None,
            expires_at: observed,
            will_renew: None,
            billing_period: None,
            billing_currency: None,
            observed_at: observed,
        }),
    });
    let value = serde_json::to_value(response).unwrap();
    assert!(value["profile"].is_null());
    assert_eq!(value["profileError"], "上游服务请求失败");
    assert_eq!(
        value["subscription"]["expiresAt"],
        "2026-09-14T00:00:00+00:00"
    );
    assert_eq!(value.as_object().unwrap().len(), 3);
}

#[test]
fn personal_info_response_preserves_unknown_subscription_and_profile_error() {
    let response = AccountPersonalInfoData::from(AccountPersonalInfo {
        profile: Err(AdminError::unavailable("Provider 服务暂不可用")),
        subscription: None,
    });
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "profile": null,
            "profileError": "Provider 服务暂不可用",
            "subscription": null,
        })
    );
}
