use gateway_core::{
    account::ProviderAccountId,
    provider_ports::{ProviderSessionTicket, ProviderSessionTicketPort},
};
use gateway_store::redis::RedisSessionTicketRepository;

#[tokio::test]
async fn tickets_survive_repository_restart_keep_absolute_expiry_and_isolate_models() {
    let Some(url) = crate::support::test_env("CPR_TEST_REDIS_URL") else {
        return;
    };
    let connection = redis::Client::open(url)
        .unwrap()
        .get_connection_manager()
        .await
        .unwrap();
    let namespace = format!("session-ticket-test-{}", uuid::Uuid::new_v4());
    let first = RedisSessionTicketRepository::new(connection.clone(), &namespace).unwrap();
    let account = ProviderAccountId::new("acct_ticket_test").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 60;
    let ticket = ProviderSessionTicket {
        value: format!("gAAAAA{}", "A".repeat(286)),
        credential_revision: 7,
        expires_at: expiry,
    };
    first.store(&account, "model-a", &ticket).await.unwrap();
    drop(first);
    let second = RedisSessionTicketRepository::new(connection.clone(), &namespace).unwrap();
    let loaded = second.load(&account, "model-a").await.unwrap().unwrap();
    assert_eq!(loaded.value, ticket.value);
    assert_eq!(loaded.expires_at, expiry);
    assert_eq!(loaded.credential_revision, 7);
    assert!(second.load(&account, "model-b").await.unwrap().is_none());
    assert!(
        second
            .load(
                &ProviderAccountId::new("acct_other_ticket").unwrap(),
                "model-a"
            )
            .await
            .unwrap()
            .is_none()
    );
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("{namespace}:*"))
        .query_async(&mut connection.clone())
        .await
        .unwrap();
    assert_eq!(keys.len(), 1);
    assert!(!keys[0].contains(account.as_str()));
    let ttl: i64 = redis::cmd("TTL")
        .arg(&keys[0])
        .query_async(&mut connection.clone())
        .await
        .unwrap();
    assert!((1..=60).contains(&ttl));
    let short = ProviderSessionTicket {
        expires_at: chrono::Utc::now().timestamp() + 10,
        ..ticket.clone()
    };
    second.store(&account, "model-b", &short).await.unwrap();
    let absolute: i64 = redis::cmd("EXPIRETIME")
        .arg(&keys[0])
        .query_async(&mut connection.clone())
        .await
        .unwrap();
    assert_eq!(absolute, expiry);
    let expired = ProviderSessionTicket {
        expires_at: chrono::Utc::now().timestamp() - 1,
        ..ticket
    };
    assert!(second.store(&account, "model-a", &expired).await.is_err());
    second.clear(&account).await.unwrap();
    assert!(second.load(&account, "model-a").await.unwrap().is_none());
    assert!(second.load(&account, "model-b").await.unwrap().is_none());
}
