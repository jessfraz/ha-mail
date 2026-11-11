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
            .arg(env!("CARGO_PKG_VERSION"))
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the project's Playwright environment"]
async fn browser_follows_automatic_sync_without_scheduling_provider_requests() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;

    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("automatic-mail.sqlite3"),
        Environment::Sandbox,
        None,
    )
    .await
    .unwrap();
    let items = (0..60)
        .map(|index| {
            let mut item =
                support::item(&format!("mail-{index:03}"), "location-a", Some("completed"));
            item.sender = format!("Fictional sender {index:03}");
            item
        })
        .collect();
    let fixture = support::Fixture::new(items);
    let stable = Stable::with_origin("fixture-key", &fixture.origin).unwrap();
    let context = api::Context::new(
        store.clone(),
        stable.clone(),
        Environment::Sandbox,
        &"test-owner".repeat(4),
    );
    let native = api::start("127.0.0.1:0".parse().unwrap(), Arc::new(context)).unwrap();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let worker_store = store.clone();
    let worker = tokio::spawn(async move {
        ha_mail::sync::worker(worker_store, stable, 300, stopped)
            .await
            .unwrap();
    });
    let origin = format!("http://{}", native.local_addr());
    let python = std::env::var("MAIL_BROWSER_PYTHON").unwrap_or_else(|_| "python3".into());
    let runtime = tokio::runtime::Handle::current();
    let browser_store = store.clone();
    let output = tokio::task::spawn_blocking(move || {
        let mut child = std::process::Command::new(python)
            .arg("tests_web/browser.py")
            .arg("--automatic")
            .arg(origin)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        for line in BufReader::new(child.stdout.take().unwrap()).lines() {
            assert_eq!(line.unwrap(), "update-fictional-mail");
            runtime.block_on(async {
                let mut item = support::item("mail-059", "location-a", Some("completed"))
                    .snapshot(200)
                    .unwrap();
                item.sender = "Fictional sender 059".into();
                item.summary = Some("Updated by the next server sync".into());
                browser_store
                    .ingest_page("location-a", &[item], true, None, true)
                    .await
                    .unwrap();
            });
            writeln!(input, "updated").unwrap();
            input.flush().unwrap();
        }
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();
    stop.send(true).unwrap();
    worker.await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    native.close().await.unwrap();
    store.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the project's Playwright environment"]
async fn browser_location_discovery_is_explicit_and_owned() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("onboarding.sqlite3"),
        Environment::Sandbox,
        None,
    )
    .await
    .unwrap();
    let fixture = support::Fixture::new(vec![]);
    let context = api::Context::new(
        store.clone(),
        Stable::with_origin("fixture-key", &fixture.origin).unwrap(),
        Environment::Sandbox,
        &"test-owner".repeat(4),
    );
    let native = api::start("127.0.0.1:0".parse().unwrap(), Arc::new(context)).unwrap();
    let origin = format!("http://{}", native.local_addr());
    let python = std::env::var("MAIL_BROWSER_PYTHON").unwrap_or_else(|_| "python3".into());
    let failures = tokio::task::spawn_blocking(move || {
        let mut failures = Vec::new();
        for scenario in ["empty", "unavailable", "stalled"] {
            let output = std::process::Command::new(&python)
                .arg("tests_web/browser.py")
                .arg("--onboarding")
                .arg(&origin)
                .arg(scenario)
                .output()
                .unwrap();
            if !output.status.success() {
                failures.push(format!(
                    "{scenario}: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
        failures
    })
    .await
    .unwrap();
    native.close().await.unwrap();
    store.close().await.unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
