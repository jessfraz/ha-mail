mod support;
use ha_mail::{
    models::{Environment, ListQuery, MailItem, OperationState},
    stable::Stable,
    store::Store,
    sync::{sweep, worker},
};
use support::{Fixture, item};

#[tokio::test]
async fn automatic_sync_waits_for_a_location_then_repeats_without_a_manual_request() {
    use std::time::Duration;

    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("mail.sqlite3"),
        Environment::Sandbox,
        None,
    )
    .await
    .unwrap();
    let fixture = Fixture::new(vec![item("mail-a", "location-a", Some("processing"))]);
    let stable = Stable::with_origin("fixture-key", &fixture.origin).unwrap();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(worker(store.clone(), stable, 300, stopped));

    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert!(fixture.requests.lock().unwrap().is_empty());
    store
        .bind(Environment::Sandbox, "location-a")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        while !store.sync_status().await.unwrap().initial_sync_complete {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(store.summary().await.unwrap().total, 1);
    assert_eq!(fixture.requests.lock().unwrap().len(), 2);
    let first_success = store.sync_status().await.unwrap().last_success_at;

    *fixture.items.lock().unwrap() = vec![
        item("mail-a", "location-a", Some("completed")),
        item("mail-b", "location-a", None),
    ];

    // Advance only while no provider request is in flight. Real HTTP/SQLite
    // work resumes on the ordinary clock, so fixture I/O cannot lose a race
    // against an automatically advanced request timeout.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(270)).await;
    tokio::time::resume();
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(fixture.requests.lock().unwrap().len(), 2);

    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(40)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(4), async {
        while store.sync_status().await.unwrap().last_success_at == first_success {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.requests.lock().unwrap().len(), 4);
    assert_eq!(store.summary().await.unwrap().total, 2);
    assert_eq!(store.summary().await.unwrap().ready_scans, 1);
    assert_eq!(
        store.binding().await.unwrap().unwrap().location_id,
        "location-a"
    );
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.starts_with("GET ")
                && request.contains("locationId=location-a"))
    );
    stop.send(true).unwrap();
    task.await.unwrap().unwrap();
    store.close().await.unwrap();
}

#[tokio::test]
async fn provider_sqlite_restart_and_late_scan_updates_preserve_binding_and_events() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite3");
    let fixture = Fixture::new(vec![item("mail-a", "location-a", None)]);
    let stable = Stable::with_origin("fixture-key", &fixture.origin).unwrap();
    let store = Store::open(&path, Environment::Sandbox, Some("location-a"))
        .await
        .unwrap();
    sweep(&store, &stable).await.unwrap();
    assert_eq!(store.summary().await.unwrap().unread, 1);
    assert!(store.sync_status().await.unwrap().initial_sync_complete);
    assert!(
        store.events(0, None).await.unwrap().items.is_empty(),
        "initial mail must not replay old notifications"
    );
    let first = store.item("mail-a").await.unwrap();
    let binding = store.binding().await.unwrap().unwrap();
    fixture.items.lock().unwrap()[0] = item("mail-a", "location-a", Some("completed"));
    sweep(&store, &stable).await.unwrap();
    let second = store.item("mail-a").await.unwrap();
    assert_eq!(second.first_seen_at, first.first_seen_at);
    assert_eq!(second.revision, first.revision + 1);
    assert_eq!(
        store.events(0, None).await.unwrap().items[0].kind,
        ha_mail::models::EventKind::ScanReady
    );
    sweep(&store, &stable).await.unwrap();
    let latest = store.item("mail-a").await.unwrap();
    // A repeated observation can advance the timestamp while the item stays unchanged.
    assert_eq!(
        latest,
        MailItem {
            observed_at: latest.observed_at,
            ..second
        }
    );
    assert_eq!(store.events(0, None).await.unwrap().items.len(), 1);
    store.close().await.unwrap();
    let reopened = Store::open(&path, Environment::Sandbox, Some("location-a"))
        .await
        .unwrap();
    assert_eq!(reopened.binding().await.unwrap().unwrap(), binding);
    assert_eq!(reopened.item("mail-a").await.unwrap(), latest);
    reopened.close().await.unwrap();
    assert!(
        Store::open(&path, Environment::Sandbox, Some("location-b"))
            .await
            .is_err()
    );
    assert!(
        Store::open(&path, Environment::Production, Some("location-a"))
            .await
            .is_err()
    );
    for request in fixture.requests.lock().unwrap().iter() {
        assert!(request.starts_with("GET "));
        assert!(request.contains("x-api-key: fixture-key"));
        if request.starts_with("GET /v1/mail-items?") {
            assert!(request.contains("locationId=location-a"));
            assert!(request.contains("archived="));
        }
    }
}

#[tokio::test]
async fn foreign_location_page_is_rejected_before_any_persistence() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("mail.sqlite3"),
        Environment::Sandbox,
        Some("location-a"),
    )
    .await
    .unwrap();
    let fixture = Fixture::new(vec![
        item("mail-a", "location-a", None),
        item("mail-b", "location-b", None),
    ]);
    let error = sweep(
        &store,
        &Stable::with_origin("fixture-key", &fixture.origin).unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "provider_location_mismatch");
    assert_eq!(store.summary().await.unwrap().total, 0);
    assert!(!store.sync_status().await.unwrap().initial_sync_complete);
}

#[tokio::test]
async fn failure_preserves_previous_data_and_rate_limit_hint() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("mail.sqlite3"),
        Environment::Sandbox,
        Some("location-a"),
    )
    .await
    .unwrap();
    let fixture = Fixture::new(vec![item("mail-a", "location-a", None)]);
    let stable = Stable::with_origin("fixture-key", &fixture.origin).unwrap();
    sweep(&store, &stable).await.unwrap();
    *fixture.fault.lock().unwrap() = Some((429, "private upstream text".into()));
    let error = sweep(&store, &stable).await.unwrap_err();
    assert_eq!(error.code, "provider_rate_limited");
    assert_eq!(error.retry_after, Some(1800));
    store.failed(error.code).await.unwrap();
    assert_eq!(store.summary().await.unwrap().total, 1);
    assert_eq!(
        store.sync_status().await.unwrap().error_code.as_deref(),
        Some("provider_rate_limited")
    );
}

#[tokio::test]
async fn manual_sync_waits_for_provider_retry_after_but_bypasses_normal_poll_delay() {
    use std::time::Duration;
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("mail.sqlite3"),
        Environment::Sandbox,
        Some("location-a"),
    )
    .await
    .unwrap();
    let fixture = Fixture::new(vec![item("mail-a", "location-a", None)]);
    *fixture.fault.lock().unwrap() = Some((429, "{}".into()));
    *fixture.retry_after.lock().unwrap() = 60;
    let stable = Stable::with_origin("fixture-key", &fixture.origin).unwrap();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(worker(store.clone(), stable, 3600, stopped));
    tokio::time::timeout(Duration::from_secs(5), async {
        while store.sync_status().await.unwrap().error_code.is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let operation = store.enqueue("manual-after-429").await.unwrap();
    *fixture.fault.lock().unwrap() = None;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let requests_during_backoff = fixture.requests.lock().unwrap().len();
    let queued_state = store.operation(operation.id).await.unwrap().state;
    if requests_during_backoff != 1 || queued_state != OperationState::Queued {
        stop.send(true).unwrap();
        task.await.unwrap().unwrap();
        panic!(
            "queued manual sync bypassed Retry-After: {requests_during_backoff} requests, {queued_state:?}"
        );
    }
    tokio::time::timeout(Duration::from_secs(62), async {
        while store.operation(operation.id).await.unwrap().state != OperationState::Completed {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.requests.lock().unwrap().len(), 3);
    {
        let times = fixture.request_times.lock().unwrap();
        assert!(times[1].duration_since(times[0]) >= Duration::from_secs(60));
    }
    assert!(store.sync_status().await.unwrap().initial_sync_complete);
    let next = store.enqueue("manual-after-success").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while store.operation(next.id).await.unwrap().state != OperationState::Completed {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.requests.lock().unwrap().len(), 5);
    stop.send(true).unwrap();
    task.await.unwrap().unwrap();
    store.close().await.unwrap();
}

#[tokio::test]
async fn keyset_pages_filters_and_same_idempotency_key_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite3");
    let store = Store::open(&path, Environment::Sandbox, Some("location-a"))
        .await
        .unwrap();
    let nodes = [
        item("mail-c", "location-a", None),
        item("mail-b", "location-a", Some("processing")),
        item("mail-a", "location-a", Some("completed")),
    ];
    let snapshots = nodes
        .iter()
        .map(|n| n.snapshot(100).unwrap())
        .collect::<Vec<_>>();
    store
        .ingest_page("location-a", &snapshots, true, None, true)
        .await
        .unwrap();
    let page = store
        .list(&ListQuery {
            limit: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        page.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        vec!["mail-c", "mail-b"]
    );
    let last = store
        .list(&ListQuery {
            page_token: page.next_page_token,
            limit: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(last.items[0].id, "mail-a");
    assert!(last.next_page_token.is_none());
    assert_eq!(
        store
            .list(&ListQuery {
                status: Some("processing".into()),
                query: Some("example".into()),
                ..Default::default()
            })
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(
        store
            .list(&ListQuery {
                limit: Some(0),
                ..Default::default()
            })
            .await
            .is_err()
    );
    let operation = store.enqueue("one-request").await.unwrap();
    assert_eq!(store.enqueue("one-request").await.unwrap().id, operation.id);
    assert!(store.enqueue("another-request").await.is_err());
    let claimed = store.claim().await.unwrap().unwrap();
    assert_eq!(claimed.state, ha_mail::models::OperationState::Running);
    store.close().await.unwrap();
    let store = Store::open(&path, Environment::Sandbox, None)
        .await
        .unwrap();
    assert_eq!(store.claim().await.unwrap().unwrap().id, operation.id);
    store.finish(operation.id, None).await.unwrap();
    assert_eq!(
        store.enqueue("one-request").await.unwrap().state,
        ha_mail::models::OperationState::Completed
    );
}

#[tokio::test]
async fn cold_backup_restores_records_binding_and_page_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite3");
    let store = Store::open(&path, Environment::Sandbox, Some("location-a"))
        .await
        .unwrap();
    let item = item("mail-a", "location-a", Some("completed"))
        .snapshot(100)
        .unwrap();
    store
        .ingest_page(
            "location-a",
            std::slice::from_ref(&item),
            false,
            Some("page-two"),
            false,
        )
        .await
        .unwrap();
    let binding = store.binding().await.unwrap();
    store.close().await.unwrap();
    let backup = directory.path().join("restored.sqlite3");
    std::fs::copy(&path, &backup).unwrap();
    let restored = Store::open(&backup, Environment::Sandbox, None)
        .await
        .unwrap();
    assert_eq!(restored.binding().await.unwrap(), binding);
    assert_eq!(restored.item("mail-a").await.unwrap(), item);
    assert_eq!(
        restored.checkpoint().await.unwrap(),
        (false, Some("page-two".into()))
    );
}

#[tokio::test]
async fn pagination_cycle_and_null_scan_are_handled_without_guessing_state() {
    let body = r#"{"id":"mail-a","from":"Example","location":{"id":"location-a","address":{"line1":"Example","line2":null,"city":"Example","state":"EX","postalCode":"00000"}},"scanDetails":null,"imageUrl":null,"clearAt":null,"newProviderField":"additive"}"#;
    let item: ha_mail::stable::ProviderItem = serde_json::from_str(body).unwrap();
    let snapshot = item.snapshot(100).unwrap();
    assert_eq!(snapshot.scan_status, "none");
    assert!(!snapshot.has_pdf);
    assert!(snapshot.clear_at.is_none());
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("mail.sqlite3"),
        Environment::Sandbox,
        Some("location-a"),
    )
    .await
    .unwrap();
    let fixture = Fixture::new(vec![]);
    *fixture.fault.lock().unwrap() = Some((
        200,
        serde_json::to_string(&support::connection::<ha_mail::stable::ProviderItem>(
            vec![],
            Some("cycle"),
        ))
        .unwrap(),
    ));
    assert_eq!(
        sweep(
            &store,
            &Stable::with_origin("fixture-key", &fixture.origin).unwrap()
        )
        .await
        .unwrap_err()
        .code,
        "repeated_provider_cursor"
    );
}

#[tokio::test]
async fn one_process_owns_database_and_clean_shutdown_releases_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite3");
    let store = Store::open(&path, Environment::Sandbox, Some("location-a"))
        .await
        .unwrap();
    assert!(
        Store::open(&path, Environment::Sandbox, Some("location-a"))
            .await
            .is_err()
    );
    store.close().await.unwrap();
    let restarted = Store::open(&path, Environment::Sandbox, Some("location-a"))
        .await
        .unwrap();
    restarted.close().await.unwrap();
}

#[tokio::test]
async fn archived_mail_keeps_its_deadline_until_provider_confirms_shredding() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("mail.sqlite3"),
        Environment::Sandbox,
        Some("location-a"),
    )
    .await
    .unwrap();
    let mut source = item("mail-a", "location-a", Some("completed"));
    source.archived_at = Some("2026-01-01T12:00:00Z".into());
    source.clear_at = Some((chrono::Utc::now() + chrono::Duration::days(2)).to_rfc3339());
    let first = source.snapshot(100).unwrap();
    store
        .ingest_page("location-a", &[first], true, None, true)
        .await
        .unwrap();
    assert_eq!(store.summary().await.unwrap().total, 0);
    assert_eq!(store.summary().await.unwrap().storage_due, 1);
    source.shred_details = Some(ha_mail::stable::StatusDetail {
        status: "completed".into(),
    });
    store
        .ingest_page(
            "location-a",
            &[source.snapshot(101).unwrap()],
            true,
            None,
            true,
        )
        .await
        .unwrap();
    assert_eq!(store.summary().await.unwrap().storage_due, 0);
    assert_eq!(
        store.item("mail-a").await.unwrap().shred_status.as_deref(),
        Some("completed")
    );
}
