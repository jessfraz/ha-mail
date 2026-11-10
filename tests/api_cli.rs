mod support;
use ha_mail::{
    api::{self, SyncRequest, System},
    client::Output,
    models::{Environment, ListQuery, MailItem, Page, Summary, SyncOperation},
    stable::Stable,
    store::Store,
};
use std::sync::Arc;
use support::{Fixture, item};

async fn cli(origin: String, args: Vec<&str>) -> std::process::Output {
    let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
    tokio::task::spawn_blocking(move || {
        std::process::Command::new(env!("CARGO_BIN_EXE_ha-mail"))
            .args(args)
            .env("MAIL_API_URL", origin)
            .env("MAIL_API_TOKEN", "test-owner".repeat(4))
            .output()
            .unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_reads_real_sqlite_http_and_honors_transport_boundaries() {
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
            &[item("mail-a", "location-a", Some("completed"))
                .snapshot(100)
                .unwrap()],
            true,
            None,
            true,
        )
        .await
        .unwrap();
    let fixture = Fixture::new(vec![item("mail-a", "location-a", None)]);
    let stable = Stable::with_origin("fixture-provider-key", &fixture.origin).unwrap();
    let context = Arc::new(
        api::Context::new(
            store.clone(),
            stable,
            Environment::Sandbox,
            &"test-owner".repeat(4),
        )
        .with_integration(&"test-integration".repeat(3)),
    );
    let server = api::start("127.0.0.1:0".parse().unwrap(), context).unwrap();
    let origin = format!("http://{}", server.local_addr());
    let result = cli(origin.clone(), vec!["summary"]).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output: Output<Option<Summary>> = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(output.body.unwrap().ready_scans, 1);
    let result = cli(origin.clone(), vec!["items", "list", "--query", "Example"]).await;
    assert!(result.status.success());
    let output: Output<Option<Page<MailItem>>> = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(output.body.unwrap().items[0].id, "mail-a");
    let result = cli(
        origin.clone(),
        vec![
            "--transport",
            "home-assistant",
            "configure",
            "--location-id",
            "location-b",
        ],
    )
    .await;
    assert!(!result.status.success());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("unsupported_transport_operation")
    );
    assert!(fixture.requests.lock().unwrap().is_empty());
    let result = cli(
        origin.clone(),
        vec!["sync", "request", "--idempotency-key", "same-request"],
    )
    .await;
    assert!(result.status.success());
    let output: Output<Option<SyncOperation>> = serde_json::from_slice(&result.stdout).unwrap();
    let id = output.body.unwrap().id;
    let result = cli(
        origin,
        vec!["sync", "request", "--idempotency-key", "same-request"],
    )
    .await;
    let output: Output<Option<SyncOperation>> = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(output.body.unwrap().id, id);
    server.close().await.unwrap();
    store.close().await.unwrap();
}

#[tokio::test]
async fn private_documents_require_local_identity_and_matching_provider_location() {
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
            &[item("mail-a", "location-a", Some("completed"))
                .snapshot(100)
                .unwrap()],
            true,
            None,
            true,
        )
        .await
        .unwrap();
    let fixture = Fixture::new(vec![item("mail-a", "location-b", Some("completed"))]);
    let context = Arc::new(
        api::Context::new(
            store.clone(),
            Stable::with_origin("fixture-key", &fixture.origin).unwrap(),
            Environment::Sandbox,
            &"test-owner".repeat(4),
        )
        .with_integration(&"test-integration".repeat(3)),
    );
    let server = api::start("127.0.0.1:0".parse().unwrap(), context.clone()).unwrap();
    let origin = format!("http://{}", server.local_addr());
    let http = reqwest::Client::new();
    assert_eq!(
        http.get(format!("{origin}/health"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        http.get(format!("{origin}/v1/items"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.get(format!("{origin}/v1/items"))
            .bearer_auth("test-integration".repeat(3))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let summary = http
        .get(format!("{origin}/v1/summary"))
        .bearer_auth("test-integration".repeat(3))
        .send()
        .await
        .unwrap();
    assert_eq!(summary.headers()["cache-control"], "no-store");
    assert_eq!(summary.status(), 200);
    let result = http
        .get(format!("{origin}/v1/items/unknown/pdf"))
        .bearer_auth("test-owner".repeat(4))
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), 404);
    assert!(fixture.requests.lock().unwrap().is_empty());
    let result = http
        .get(format!("{origin}/v1/items/mail-a/pdf"))
        .bearer_auth("test-owner".repeat(4))
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), 503);
    let body = result.text().await.unwrap();
    assert!(!body.contains("location-b"));
    assert!(!body.contains("fixture-key"));
    let ingress = api::start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(
            (*context)
                .clone()
                .for_ingress("203.0.113.1".parse().unwrap()),
        ),
    )
    .unwrap();
    assert_eq!(
        http.get(format!("http://{}/", ingress.local_addr()))
            .header("x-remote-user-id", "forged")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    server.close().await.unwrap();
    ingress.close().await.unwrap();
}

#[tokio::test]
async fn selecting_location_is_explicit_and_cannot_retarget_existing_database() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("mail.sqlite3"),
        Environment::Sandbox,
        None,
    )
    .await
    .unwrap();
    let fixture = Fixture::new(vec![]);
    let server = api::start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(api::Context::new(
            store.clone(),
            Stable::with_origin("fixture-key", &fixture.origin).unwrap(),
            Environment::Sandbox,
            &"test-owner".repeat(4),
        )),
    )
    .unwrap();
    let origin = format!("http://{}", server.local_addr());
    let http = reqwest::Client::new();
    let token = "test-owner".repeat(4);
    assert!(
        http.get(format!("{origin}/v1/system"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json::<System>()
            .await
            .unwrap()
            .binding
            .is_none()
    );
    let request = ha_mail::api::SelectLocation {
        location_id: "location-a".into(),
    };
    assert_eq!(
        http.post(format!("{origin}/v1/installation/location"))
            .bearer_auth(&token)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        http.post(format!("{origin}/v1/installation/location"))
            .bearer_auth(&token)
            .json(&ha_mail::api::SelectLocation {
                location_id: "location-b".into()
            })
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    let sync = SyncRequest {
        idempotency_key: "read-only-sync".into(),
    };
    assert_eq!(
        http.post(format!("{origin}/v1/sync"))
            .bearer_auth(token)
            .json(&sync)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        store.binding().await.unwrap().unwrap().location_id,
        "location-a"
    );
    assert!(
        store
            .list(&ListQuery::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_sync_requests_have_one_owned_operation() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("mail.sqlite3"),
        Environment::Sandbox,
        Some("location-a"),
    )
    .await
    .unwrap();
    let (first, second) = tokio::join!(store.enqueue("same-key"), store.enqueue("same-key"));
    assert_eq!(first.unwrap().id, second.unwrap().id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_writes_owner_only_pdf_bytes_without_overwriting() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new(vec![]);
    let pdf = "%PDF-1.7\nFictional test document\n%%EOF\n";
    *fixture.fault.lock().unwrap() = Some((200, pdf.into()));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail-scan.pdf");
    let origin = fixture.origin.clone();
    let command_path = path.to_string_lossy().into_owned();
    let output = cli(
        origin.clone(),
        vec!["items", "pdf", "mail-a", "--output", &command_path],
    )
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(&path).unwrap(), pdf.as_bytes());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let second = cli(
        origin,
        vec!["items", "pdf", "mail-a", "--output", &command_path],
    )
    .await;
    assert!(!second.status.success());
    assert!(
        String::from_utf8(second.stderr)
            .unwrap()
            .contains("output_exists")
    );
    assert_eq!(std::fs::read(&path).unwrap(), pdf.as_bytes());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}
