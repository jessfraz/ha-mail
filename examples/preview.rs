//! A disposable fictional mailbox for local UI review. No ingestion worker or
//! real provider credential is constructed, so this cannot fetch household mail.
use ha_mail::{
    api,
    models::{Environment, MailItem},
    stable::Stable,
    store::Store,
};
use std::sync::Arc;
#[path = "../tests/support/mod.rs"]
mod support;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(
        &directory.path().join("mail.sqlite3"),
        Environment::Sandbox,
        Some("example-location"),
    )
    .await?;
    let now = chrono::Utc::now().timestamp();
    if std::env::var("MAIL_PREVIEW_SCENARIO").as_deref() != Ok("empty") {
        let items = [
            (
                "example-letter",
                "Example Studio",
                "completed",
                Some("A fictional project update for the demo mailbox."),
            ),
            ("example-notice", "Example Services", "processing", None),
            ("example-envelope", "Example Correspondence", "none", None),
        ]
        .into_iter()
        .map(|(id, sender, status, summary)| MailItem {
            id: id.into(),
            sender: sender.into(),
            received_at: Some("2026-01-01T12:00:00Z".into()),
            first_seen_at: now,
            observed_at: now,
            archived_at: None,
            read_at: None,
            clear_at: Some((chrono::Utc::now() + chrono::Duration::days(2)).to_rfc3339()),
            scan_status: status.into(),
            scan_notice: None,
            shred_status: None,
            summary: summary.map(str::to_owned),
            has_envelope: false,
            has_pdf: false,
            has_ocr: false,
            revision: 1,
        })
        .collect::<Vec<_>>();
        store
            .ingest_page("example-location", &items, true, None, true)
            .await?;
    }
    if std::env::var("MAIL_PREVIEW_SCENARIO").as_deref() == Ok("error") {
        store.failed("provider_unavailable").await?;
    }
    let fixture = support::Fixture::new(vec![]);
    let context = api::Context::new(
        store.clone(),
        Stable::with_origin("fictional-preview-only", &fixture.origin)?,
        Environment::Sandbox,
        &"fictional-preview".repeat(3),
    );
    let server = api::start(
        std::net::SocketAddr::from((
            [127, 0, 0, 1],
            std::env::var("MAIL_PREVIEW_PORT")
                .ok()
                .map(|s| s.parse())
                .transpose()?
                .unwrap_or(0),
        )),
        Arc::new(context),
    )?;
    println!("Fictional preview: http://{}", server.local_addr());
    println!("Preview-only token: {}", "fictional-preview".repeat(3));
    tokio::signal::ctrl_c().await?;
    server
        .close()
        .await
        .map_err(|_| anyhow::anyhow!("close preview server"))?;
    store.close().await?;
    Ok(())
}
