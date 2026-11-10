mod support;
use ha_mail::{api, models::Environment, stable::Stable, store::Store};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the project's Playwright environment"]
async fn browser_exercises_real_mail_dashboard_and_ingress_paths() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("mail.sqlite3"),
        Environment::Sandbox,
        Some("location-a"),
    )
    .await
    .unwrap();
    let mut item = support::item("mail-a", "location-a", Some("completed"))
        .snapshot(100)
        .unwrap();
    item.summary = Some("<img src=x onerror=alert(1)>".into());
    item.has_envelope = true;
    item.has_pdf = true;
    item.has_ocr = true;
    store
        .ingest_page("location-a", &[item], true, None, true)
        .await
        .unwrap();
    let fixture = support::Fixture::new(vec![]);
    let context = api::Context::new(
        store.clone(),
        Stable::with_origin("fixture-key", &fixture.origin).unwrap(),
        Environment::Sandbox,
        &"test-owner".repeat(4),
    );
    let native = api::start("127.0.0.1:0".parse().unwrap(), Arc::new(context.clone())).unwrap();
    let ingress = api::start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(context.for_ingress("127.0.0.1".parse().unwrap())),
    )
    .unwrap();
    let args = [
        format!("http://{}", native.local_addr()),
        format!("http://{}", ingress.local_addr()),
    ];
    let python = std::env::var("MAIL_BROWSER_PYTHON").unwrap_or_else(|_| "python3".into());
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(python)
            .arg("tests_web/browser.py")
            .args(args)
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
    native.close().await.unwrap();
    ingress.close().await.unwrap();
    store.close().await.unwrap();
}
