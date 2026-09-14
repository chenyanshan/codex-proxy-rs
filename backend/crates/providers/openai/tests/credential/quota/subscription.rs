use super::*;

#[tokio::test]
async fn delayed_subscription_preserves_newer_quota_but_not_a_new_credential_revision() {
    for rotate_credential in [false, true] {
        let store = Arc::new(MemoryAccountStore::default());
        create_account(&store, "acct_concurrent_subscription").await;
        let account = store.account("acct_concurrent_subscription").unwrap();
        let server = MockServer::start().await;
        let service = quota_service_with_base_url(
            &store,
            reqwest::Client::builder().no_proxy().build().unwrap(),
            server.uri(),
        );
        Mock::given(path("/api/codex/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"rate_limit":{"allowed":true,"primary_window":{"used_percent":10}}}),
            ))
            .mount(&server)
            .await;
        Mock::given(path("/backend-api/subscriptions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"active_until":"2026-12-01T00:00:00Z"}))
                    .set_delay(std::time::Duration::from_millis(250)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let competing_write = async {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    if server
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .any(|request| request.url.path() == "/backend-api/subscriptions")
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            if rotate_credential {
                let repository = store.repository();
                let data = repository.load_complete_data(&account).await.unwrap();
                repository
                    .compare_and_swap_data(&account, data)
                    .await
                    .unwrap();
            } else {
                let event=parse_rate_limits_event(&json!({"type":"codex.rate_limits","rate_limits":{"limit_id":"codex","primary":{"used_percent":80,"window_minutes":300}}})).unwrap();
                assert!(
                    service
                        .synchronize_passive_rate_limits(&account, &[event])
                        .await
                        .unwrap()
                );
            }
        };
        let (result, ()) = tokio::join!(service.refresh_account(account.id()), competing_write);
        if rotate_credential {
            assert!(matches!(
                result,
                Err(CodexCredentialQuotaError::RevisionConflict)
            ));
        } else {
            let snapshot = result.unwrap();
            assert_eq!(snapshot.fact().remaining_percent(), Some(20));
            assert!(snapshot.subscription().is_none());
            assert_eq!(
                service
                    .read_account(account.id())
                    .await
                    .unwrap()
                    .unwrap()
                    .observed_at(),
                snapshot.observed_at()
            );
        }
    }
}

#[tokio::test]
async fn malformed_legacy_subscription_does_not_poison_quota() {
    use gateway_core::account::{OpaqueProviderData, QuotaObservation};
    let store = Arc::new(MemoryAccountStore::default());
    create_account(&store, "acct_legacy_subscription").await;
    let account = store.account("acct_legacy_subscription").unwrap();
    for invalid in [
        json!(null),
        json!({}),
        json!({"account_id":"chatgpt-acct_legacy_subscription","expires_at":"bad","observed_at":"2026-01-01T00:00:00Z"}),
    ] {
        store.compare_and_swap_quota(QuotaObservation {
            account_id: account.id().clone(), expected_revision: account.revision(),
            observed_at: SystemTime::now(), state: QuotaState::allowed(SystemTime::now()),
            quota: OpaqueProviderData::new(json!({"subscription":invalid,"rate_limit":{"primary_window":{"used_percent":25}}}).as_object().unwrap().clone()),
        }).await.unwrap();
        let snapshot = quota_service(&store)
            .read_account(account.id())
            .await
            .unwrap()
            .unwrap();
        assert!(snapshot.subscription().is_none());
        assert_eq!(snapshot.fact().remaining_percent(), Some(75));
    }
}

#[tokio::test]
async fn active_refresh_and_passive_observation_keep_subscription_independent() {
    let store = Arc::new(MemoryAccountStore::default());
    create_account(&store, "acct_subscription").await;
    let account = store.account("acct_subscription").unwrap();
    let server = MockServer::start().await;
    let service = quota_service_with_base_url(
        &store,
        reqwest::Client::builder().no_proxy().build().unwrap(),
        server.uri(),
    );
    let usage = json!({"plan_type":"plus","rate_limit":{"allowed":true,"primary_window":{"used_percent":10}},"subscription":{"expires_at":"2099-01-01T00:00:00Z"}});
    Mock::given(path("/api/codex/usage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(usage.clone()))
        .mount(&server)
        .await;
    Mock::given(path("/backend-api/subscriptions"))
        .and(wiremock::matchers::query_param("account_id","chatgpt-acct_subscription"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"plan_type":"enterprise","active_until":"2020-01-01T00:00:00Z","will_renew":true})))
        .expect(1).mount(&server).await;
    let snapshot = service.refresh_account(account.id()).await.unwrap();
    assert_eq!(snapshot.plan_type(), Some("plus"));
    assert_eq!(snapshot.subscription().unwrap().will_renew, Some(true));
    assert_eq!(
        store
            .account("acct_subscription")
            .unwrap()
            .credential_state(),
        account.credential_state()
    );
    let before = store.quota_json("acct_subscription").unwrap()["subscription"].clone();
    assert!(before.is_object());
    let event=parse_rate_limits_event(&json!({"type":"codex.rate_limits","rate_limits":{"limit_id":"codex","primary":{"used_percent":20,"window_minutes":300}}})).unwrap();
    service
        .synchronize_passive_rate_limits(&account, &[event])
        .await
        .unwrap();
    assert_eq!(
        store.quota_json("acct_subscription").unwrap()["subscription"],
        before
    );
    let requests = server.received_requests().await.unwrap().len();
    assert!(
        service
            .read_account(account.id())
            .await
            .unwrap()
            .unwrap()
            .subscription()
            .is_some()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), requests);
    server.reset().await;
    Mock::given(path("/api/codex/usage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(usage))
        .mount(&server)
        .await;
    Mock::given(path("/backend-api/subscriptions"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    let unknown = service.refresh_account(account.id()).await.unwrap();
    assert!(unknown.subscription().is_none());
    assert_eq!(unknown.plan_type(), Some("plus"));
    assert!(
        store
            .quota_json("acct_subscription")
            .unwrap()
            .get("subscription")
            .is_none()
    );
    assert_eq!(
        store
            .account("acct_subscription")
            .unwrap()
            .credential_state(),
        account.credential_state()
    );
}
