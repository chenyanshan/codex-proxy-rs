//! 在独立测试进程中验证共享建连准入，避免其他网络测试占用全局名额。

use super::*;
use gateway_core::account::OutboundProxy;
use provider_openai::transport::client::build_account_http_client;

#[tokio::test]
async fn cold_connection_admission_bounds_queue_and_releases_cancelled_work() {
    const CHILD: &str = "CPR_CONNECTION_ADMISSION_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "transport::connection::cold_connection_admission_bounds_queue_and_releases_cancelled_work", "--nocapture"])
            .env(CHILD, "1")
            .status().unwrap();
        assert!(status.success());
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy =
        OutboundProxy::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let client = build_account_http_client("acct_connection_admission", Some(&proxy)).unwrap();
    let (completed, mut results) = tokio::sync::mpsc::unbounded_channel();
    let mut tasks = Vec::new();
    let spawn = |client: reqwest::Client, completed: tokio::sync::mpsc::UnboundedSender<bool>| {
        tokio::spawn(async move {
            let result = client.get("https://upstream.invalid/").send().await;
            let _ = completed.send(result.is_err_and(|error| error.is_connect()));
        })
    };
    for _ in 0..128 {
        tasks.push(spawn(client.clone(), completed.clone()));
    }
    let mut sockets = Vec::new();
    for _ in 0..128 {
        sockets.push(
            timeout(Duration::from_secs(10), listener.accept())
                .await
                .unwrap()
                .unwrap()
                .0,
        );
    }
    // 128 条 CONNECT 握手保持挂起；1024 个等待者之外的一次请求应立即拒绝。
    for _ in 0..1025 {
        tasks.push(spawn(client.clone(), completed.clone()));
    }
    assert_eq!(
        timeout(Duration::from_secs(3), results.recv())
            .await
            .unwrap(),
        Some(true)
    );
    assert!(
        timeout(Duration::from_millis(100), results.recv())
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
    crate::provider::assert_local_connection_capacity_is_not_an_upstream_failure().await;
    tasks[0].abort();
    sockets.push(
        timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap()
            .0,
    );
    for task in &tasks {
        task.abort();
    }
    for task in tasks {
        let _ = task.await;
    }
    // 同时取消活动与排队请求后，新请求仍能取得名额。
    let fresh = spawn(client, completed);
    let _socket = timeout(Duration::from_secs(3), listener.accept())
        .await
        .unwrap()
        .unwrap();
    fresh.abort();
    let _ = fresh.await;
}
