use crate::{stable::Stable, store::Store};
use std::collections::HashSet;

#[derive(Debug)]
pub struct SyncFailure {
    pub code: &'static str,
    pub retry_after: Option<u64>,
}
impl From<crate::stable::ProviderError> for SyncFailure {
    fn from(error: crate::stable::ProviderError) -> Self {
        Self {
            code: error.code,
            retry_after: error.retry_after,
        }
    }
}
impl From<&'static str> for SyncFailure {
    fn from(code: &'static str) -> Self {
        Self {
            code,
            retry_after: None,
        }
    }
}
pub async fn sweep(store: &Store, stable: &Stable) -> Result<(), SyncFailure> {
    let binding = store
        .binding()
        .await
        .map_err(|_| "storage_unavailable")?
        .ok_or("location_required")?;
    let (mut archived, mut cursor) = store
        .checkpoint()
        .await
        .map_err(|_| "storage_unavailable")?;
    let mut cursors = HashSet::new();
    loop {
        let page = stable
            .page(&binding.location_id, archived, cursor.as_deref())
            .await
            .map_err(SyncFailure::from)?;
        let mut items = Vec::new();
        for edge in page.edges {
            if edge.node.location.id != binding.location_id {
                return Err("provider_location_mismatch".into());
            }
            items.push(
                edge.node
                    .snapshot(chrono::Utc::now().timestamp())
                    .map_err(SyncFailure::from)?,
            );
        }
        let next = if page.page_info.has_next_page {
            let next = page
                .page_info
                .end_cursor
                .filter(|s| !s.is_empty() && s.len() <= 4096)
                .ok_or("invalid_provider_cursor")?;
            if Some(&next) == cursor.as_ref() || !cursors.insert((archived, next.clone())) {
                return Err("repeated_provider_cursor".into());
            }
            Some(next)
        } else {
            None
        };
        let complete = archived && next.is_none();
        let next_phase = archived || next.is_none();
        store
            .ingest_page(
                &binding.location_id,
                &items,
                next_phase,
                next.as_deref(),
                complete,
            )
            .await
            .map_err(|_| "storage_unavailable")?;
        if complete {
            return Ok(());
        }
        archived = next_phase;
        cursor = next;
    }
}

pub async fn worker(
    store: Store,
    stable: Stable,
    interval: u64,
    mut stopped: tokio::sync::watch::Receiver<bool>,
) -> Result<(), crate::error::Error> {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(2));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut next_poll = tokio::time::Instant::now();
    let mut provider_not_before = next_poll;
    loop {
        tokio::select! { _ = stopped.changed() => break, _ = ticker.tick() => {} }
        if *stopped.borrow() {
            break;
        }
        if store.binding().await?.is_none() {
            continue;
        }
        // Manual requests bypass the normal poll interval, never provider backoff.
        // Leave them queued until a request can actually start.
        if tokio::time::Instant::now() < provider_not_before {
            continue;
        }
        let operation = store.claim().await?;
        if operation.is_none() && tokio::time::Instant::now() < next_poll {
            continue;
        }
        let result = tokio::select! { _ = stopped.changed() => break, result = sweep(&store, &stable) => result };
        if let Err(error) = &result {
            store.failed(error.code).await?;
        }
        if let Some(operation) = operation {
            store
                .finish(operation.id, result.as_ref().err().map(|error| error.code))
                .await?;
        }
        let now = tokio::time::Instant::now();
        next_poll = now + std::time::Duration::from_secs(interval.max(60));
        provider_not_before = now
            + std::time::Duration::from_secs(
                result
                    .as_ref()
                    .err()
                    .and_then(|error| error.retry_after)
                    .unwrap_or(0),
            );
    }
    Ok(())
}
