mod support;
use ha_mail::{api, models::Environment, stable::Stable, store::Store};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the project's Home Assistant environment"]
async fn real_home_assistant_bridge_reads_rust_service_and_preserves_authorization() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("mail.sqlite3"),
        Environment::Sandbox,
        Some("location-a"),
    )
    .await
    .unwrap();
    store
        .ingest_page(
            "location-a",
            &[support::item("mail-a", "location-a", Some("completed"))
                .snapshot(100)
                .unwrap()],
            true,
            None,
            true,
        )
        .await
        .unwrap();
    let fixture = support::Fixture::new(vec![]);
    let context = api::Context::new(
        store.clone(),
        Stable::with_origin("fixture-key", &fixture.origin).unwrap(),
        Environment::Sandbox,
        &"test-owner".repeat(4),
    )
    .with_integration(&"test-integration".repeat(3));
    let server = api::start("127.0.0.1:0".parse().unwrap(), Arc::new(context)).unwrap();
    let origin = format!("http://{}", server.local_addr());
    let python = std::env::var("MAIL_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(python)
            .args(["-m", "pytest", "-q", "tests_ha"])
            .env("MAIL_TEST_ORIGIN", origin)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.close().await.unwrap();
    store.close().await.unwrap();
}
