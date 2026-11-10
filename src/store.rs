use crate::{
    error::Error,
    models::{
        Binding, Environment, Event, ListQuery, MailItem, Page, Summary, SyncOperation, SyncStatus,
        valid_id,
    },
};
use sqlx::{
    QueryBuilder, Row, Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::Path, time::Duration};

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
    lock: std::sync::Arc<std::fs::File>,
}

impl Store {
    pub async fn open(
        path: &Path,
        environment: Environment,
        location: Option<&str>,
    ) -> Result<Self, Error> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(file) => file
                .sync_all()
                .map_err(|_| Error::Invalid("cannot persist database"))?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(Error::Invalid("cannot create database")),
        }
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|_| Error::Invalid("cannot inspect database"))?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::Invalid(
                "database must be an owner-only regular file",
            ));
        }
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock_path)
            .map_err(|_| Error::Invalid("cannot lock database"))?;
        let lock_metadata = std::fs::symlink_metadata(&lock_path)
            .map_err(|_| Error::Invalid("cannot inspect database lock"))?;
        if !lock_metadata.is_file() || lock_metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::Invalid("database lock must be owner-only"));
        }
        lock.try_lock()
            .map_err(|_| Error::Invalid("database is already in use"))?;
        let options = SqliteConnectOptions::new()
            .filename(path)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let store = Self {
            lock: std::sync::Arc::new(lock),
            pool: SqlitePoolOptions::new()
                .max_connections(4)
                .connect_with(options)
                .await?,
        };
        sqlx::migrate!().run(&store.pool).await?;
        if let Some(binding) = store.binding().await? {
            if binding.environment != environment
                || location.is_some_and(|id| id != binding.location_id)
            {
                return Err(Error::Binding);
            }
        } else if let Some(location) = location {
            store.bind(environment, location).await?;
        }
        // GET-only ingestion is safe to resume, unlike an uncertain physical action.
        sqlx::query("UPDATE operations SET state = 'queued' WHERE state IN ('queued', 'running') AND state = 'running'")
            .execute(&store.pool)
            .await?;
        Ok(store)
    }
    pub async fn close(&self) -> Result<(), Error> {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await?;
        self.pool.close().await;
        self.lock
            .unlock()
            .map_err(|_| Error::Invalid("cannot release database lock"))?;
        Ok(())
    }
    pub async fn ready(&self) -> Result<(), Error> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }
    pub async fn binding(&self) -> Result<Option<Binding>, Error> {
        let row = sqlx::query(
            "SELECT instance_id, environment, location_id FROM binding WHERE singleton = 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| {
            let env: String = r.try_get("environment")?;
            let environment = match env.as_str() {
                "production" => Environment::Production,
                "sandbox" => Environment::Sandbox,
                _ => return Err(Error::Corrupt),
            };
            let instance: String = r.try_get("instance_id")?;
            Ok(Binding {
                instance_id: uuid::Uuid::parse_str(&instance).map_err(|_| Error::Corrupt)?,
                environment,
                location_id: r.try_get("location_id")?,
            })
        })
        .transpose()
    }
    pub async fn bind(&self, environment: Environment, location: &str) -> Result<Binding, Error> {
        if !valid_id(location) {
            return Err(Error::Invalid("invalid location ID"));
        }
        sqlx::query("INSERT INTO binding (singleton, instance_id, environment, location_id) VALUES (1, ?, ?, ?) ON CONFLICT(singleton) DO NOTHING")
            .bind(uuid::Uuid::now_v7().to_string()).bind(environment.as_str()).bind(location).execute(&self.pool).await?;
        let binding = self.binding().await?.ok_or(Error::Corrupt)?;
        if binding.environment != environment || binding.location_id != location {
            return Err(Error::Binding);
        }
        Ok(binding)
    }
    pub async fn item(&self, id: &str) -> Result<MailItem, Error> {
        if !valid_id(id) {
            return Err(Error::Invalid("invalid item ID"));
        }
        let body: Option<String> = sqlx::query_scalar("SELECT body FROM items WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(serde_json::from_str(&body.ok_or(Error::NotFound)?)?)
    }
    pub async fn list(&self, query: &ListQuery) -> Result<Page<MailItem>, Error> {
        let limit = query.limit.unwrap_or(50);
        let mut sql = list_query(query)?;
        let rows = sql.build().fetch_all(&self.pool).await?;
        let has_more = rows.len() > limit as usize;
        let mut items = Vec::new();
        let mut last = None;
        for row in rows.into_iter().take(limit as usize) {
            let body: String = row.try_get("body")?;
            items.push(serde_json::from_str(&body)?);
            last = Some(format!(
                "{}:{}",
                row.try_get::<i64, _>("sort_at")?,
                row.try_get::<String, _>("id")?
            ));
        }
        Ok(Page {
            items,
            next_page_token: if has_more { last } else { None },
        })
    }
    pub async fn summary(&self) -> Result<Summary, Error> {
        let row = sqlx::query("SELECT count(*) FILTER (WHERE archived = 0) AS total, coalesce(sum(unread) FILTER (WHERE archived = 0), 0) AS unread, count(*) FILTER (WHERE archived = 0 AND scan_status = 'processing') AS pending, count(*) FILTER (WHERE archived = 0 AND scan_status = 'completed') AS ready, count(*) FILTER (WHERE clear_at <= unixepoch() + 259200) AS due FROM items").fetch_one(&self.pool).await?;
        Ok(Summary {
            total: row.try_get("total")?,
            unread: row.try_get("unread")?,
            pending_scans: row.try_get("pending")?,
            ready_scans: row.try_get("ready")?,
            storage_due: row.try_get("due")?,
        })
    }
    pub async fn sync_status(&self) -> Result<SyncStatus, Error> {
        let row = sqlx::query("SELECT last_success_at, last_attempt_at, error_code, initial_sync_complete FROM sync_state WHERE singleton = 1").fetch_one(&self.pool).await?;
        Ok(SyncStatus {
            last_success_at: row.try_get("last_success_at")?,
            last_attempt_at: row.try_get("last_attempt_at")?,
            error_code: row.try_get("error_code")?,
            initial_sync_complete: row.try_get("initial_sync_complete")?,
        })
    }
    pub async fn checkpoint(&self) -> Result<(bool, Option<String>), Error> {
        let row = sqlx::query("SELECT archived_phase, cursor FROM sync_state WHERE singleton = 1")
            .fetch_one(&self.pool)
            .await?;
        Ok((row.try_get("archived_phase")?, row.try_get("cursor")?))
    }
    pub async fn ingest_page(
        &self,
        location: &str,
        items: &[MailItem],
        phase: bool,
        cursor: Option<&str>,
        complete: bool,
    ) -> Result<(), Error> {
        if items.len() > 500 || items.iter().any(|item| !valid_id(&item.id)) {
            return Err(Error::Invalid("invalid provider page"));
        }
        let ids = items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        if ids.len() != items.len() {
            return Err(Error::Invalid("duplicate provider item"));
        }
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let mut previous_items = std::collections::HashMap::new();
        if !items.is_empty() {
            let mut query = QueryBuilder::<Sqlite>::new("SELECT id, body FROM items WHERE id IN (");
            let mut separated = query.separated(", ");
            for item in items {
                separated.push_bind(&item.id);
            }
            separated.push_unseparated(")");
            for row in query.build().fetch_all(&mut *transaction).await? {
                let body: String = row.try_get("body")?;
                previous_items.insert(
                    row.try_get::<String, _>("id")?,
                    serde_json::from_str::<MailItem>(&body)?,
                );
            }
        }
        let bound: String =
            sqlx::query_scalar("SELECT location_id FROM binding WHERE singleton = 1")
                .fetch_one(&mut *transaction)
                .await?;
        if bound != location {
            return Err(Error::Binding);
        }
        let initial: bool =
            sqlx::query_scalar("SELECT initial_sync_complete FROM sync_state WHERE singleton = 1")
                .fetch_one(&mut *transaction)
                .await?;
        for snapshot in items {
            let previous = previous_items.remove(&snapshot.id);
            let mut item = snapshot.clone();
            let mut events = Vec::new();
            if let Some(old) = &previous {
                item.first_seen_at = old.first_seen_at;
                item.revision = old.revision;
                let observed = item.observed_at;
                item.observed_at = old.observed_at;
                let changed = item != *old;
                item.observed_at = observed;
                if changed {
                    item.revision = item.revision.checked_add(1).ok_or(Error::Corrupt)?;
                }
                if initial && old.scan_status != "completed" && item.scan_status == "completed" {
                    events.push("scan_ready");
                }
                if initial && old.scan_notice != item.scan_notice && item.scan_notice.is_some() {
                    events.push("scan_notice");
                }
            } else if initial {
                events.push("new_mail");
            }
            let clear_at = item
                .clear_at
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.timestamp())
                .filter(|_| item.shred_status.as_deref() != Some("completed"));
            sqlx::query("INSERT INTO items (id, sender, scan_status, summary, archived, unread, clear_at, body, sort_at, first_seen_at, revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET sender = excluded.sender, scan_status = excluded.scan_status, summary = excluded.summary, archived = excluded.archived, unread = excluded.unread, clear_at = excluded.clear_at, body = excluded.body, sort_at = excluded.sort_at, revision = excluded.revision")
                .bind(&item.id).bind(&item.sender).bind(&item.scan_status).bind(&item.summary).bind(item.archived_at.is_some()).bind(item.read_at.is_none()).bind(clear_at).bind(serde_json::to_string(&item)?).bind(item.received_at.as_deref().and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok()).map_or(item.first_seen_at, |at| at.timestamp())).bind(item.first_seen_at).bind(item.revision).execute(&mut *transaction).await?;
            if initial && clear_at.is_some_and(|at| at <= chrono::Utc::now().timestamp() + 259200) {
                events.push("storage_due");
            }
            for kind in events {
                let notification_key = if kind == "storage_due" {
                    clear_at.ok_or(Error::Corrupt)?.to_string()
                } else {
                    item.revision.to_string()
                };
                sqlx::query("INSERT INTO events (item_id, kind, revision, notification_key) VALUES (?, ?, ?, ?) ON CONFLICT DO NOTHING").bind(&item.id).bind(kind).bind(item.revision).bind(notification_key).execute(&mut *transaction).await?;
            }
        }
        if complete {
            sqlx::query("UPDATE sync_state SET archived_phase = 0, cursor = NULL, last_success_at = unixepoch(), last_attempt_at = unixepoch(), error_code = NULL, initial_sync_complete = 1 WHERE singleton = 1").execute(&mut *transaction).await?;
        } else {
            sqlx::query("UPDATE sync_state SET archived_phase = ?, cursor = ?, last_attempt_at = unixepoch() WHERE singleton = 1").bind(phase).bind(cursor).execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        Ok(())
    }
    pub async fn failed(&self, code: &str) -> Result<(), Error> {
        sqlx::query("UPDATE sync_state SET error_code = ?, last_attempt_at = unixepoch(), archived_phase = 0, cursor = NULL WHERE singleton = 1").bind(code).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn enqueue(&self, key: &str) -> Result<SyncOperation, Error> {
        if key.is_empty()
            || key.len() > 128
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(Error::Invalid("invalid idempotency key"));
        }
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(row) = sqlx::query("SELECT id, state, error_code, created_at, completed_at FROM operations WHERE idempotency_key = ?").bind(key).fetch_optional(&mut *transaction).await? { return operation(row); }
        let pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE state IN ('queued', 'running'))",
        )
        .fetch_one(&mut *transaction)
        .await?;
        if pending {
            return Err(Error::Busy);
        }
        let id = uuid::Uuid::now_v7();
        sqlx::query("INSERT INTO operations (id, idempotency_key, state) VALUES (?, ?, 'queued')")
            .bind(id.to_string())
            .bind(key)
            .execute(&mut *transaction)
            .await?;
        let row = sqlx::query(
            "SELECT id, state, error_code, created_at, completed_at FROM operations WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        operation(row)
    }
    pub async fn claim(&self) -> Result<Option<SyncOperation>, Error> {
        let row = sqlx::query("UPDATE operations SET state = 'running' WHERE id = (SELECT id FROM operations WHERE state IN ('queued', 'running') AND state = 'queued' LIMIT 1) RETURNING id, state, error_code, created_at, completed_at").fetch_optional(&self.pool).await?;
        row.map(operation).transpose()
    }
    pub async fn operation(&self, id: uuid::Uuid) -> Result<SyncOperation, Error> {
        operation(sqlx::query("SELECT id, state, error_code, created_at, completed_at FROM operations WHERE id = ?").bind(id.to_string()).fetch_optional(&self.pool).await?.ok_or(Error::NotFound)?)
    }
    pub async fn finish(&self, id: uuid::Uuid, error: Option<&str>) -> Result<(), Error> {
        sqlx::query("UPDATE operations SET state = ?, error_code = ?, completed_at = unixepoch() WHERE id = ? AND state = 'running'").bind(if error.is_some() {"failed"} else {"completed"}).bind(error).bind(id.to_string()).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn events(&self, after: i64, id: Option<&str>) -> Result<Page<Event>, Error> {
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT sequence, item_id, kind, observed_at FROM events WHERE sequence > ",
        );
        sql.push_bind(after);
        if let Some(id) = id {
            sql.push(" AND item_id = ").push_bind(id);
        }
        sql.push(" ORDER BY sequence LIMIT 201");
        let rows = sql.build().fetch_all(&self.pool).await?;
        let more = rows.len() > 200;
        let items = rows
            .into_iter()
            .take(200)
            .map(|r| {
                Ok(Event {
                    sequence: r.try_get("sequence")?,
                    item_id: r.try_get("item_id")?,
                    kind: r.try_get::<String, _>("kind")?.as_str().try_into()?,
                    observed_at: r.try_get("observed_at")?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let next_page_token = if more {
            items.last().map(|e| e.sequence.to_string())
        } else {
            None
        };
        Ok(Page {
            items,
            next_page_token,
        })
    }
    pub async fn event_head(&self) -> Result<crate::models::EventHead, Error> {
        Ok(crate::models::EventHead {
            sequence: sqlx::query_scalar("SELECT coalesce(max(sequence), 0) FROM events")
                .fetch_one(&self.pool)
                .await?,
        })
    }
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

fn operation(row: sqlx::sqlite::SqliteRow) -> Result<SyncOperation, Error> {
    let id: String = row.try_get("id")?;
    Ok(SyncOperation {
        id: uuid::Uuid::parse_str(&id).map_err(|_| Error::Corrupt)?,
        state: row.try_get::<String, _>("state")?.as_str().try_into()?,
        error_code: row.try_get("error_code")?,
        created_at: row.try_get("created_at")?,
        completed_at: row.try_get("completed_at")?,
    })
}

fn list_query(query: &ListQuery) -> Result<QueryBuilder<Sqlite>, Error> {
    let limit = query.limit.unwrap_or(50);
    if !(1..=200).contains(&limit)
        || query.query.as_ref().is_some_and(|v| v.len() > 200)
        || query.status.as_ref().is_some_and(|s| s.len() > 100)
    {
        return Err(Error::Invalid("invalid mail filters"));
    }
    let cursor = query
        .page_token
        .as_deref()
        .map(|s| {
            let (timestamp, id) = s
                .split_once(':')
                .ok_or(Error::Invalid("invalid page token"))?;
            let at = timestamp
                .parse::<i64>()
                .map_err(|_| Error::Invalid("invalid page token"))?;
            if !valid_id(id) {
                return Err(Error::Invalid("invalid page token"));
            }
            Ok((at, id))
        })
        .transpose()?;
    let mut sql =
        QueryBuilder::<Sqlite>::new("SELECT id, sort_at, body FROM items WHERE archived = ");
    sql.push_bind(query.archived.unwrap_or(false));
    if let Some(status) = &query.status {
        sql.push(" AND scan_status = ").push_bind(status);
    }
    if let Some(unread) = query.unread {
        sql.push(" AND unread = ").push_bind(unread);
    }
    if let Some(q) = query.query.as_deref().filter(|s| !s.is_empty()) {
        sql.push(" AND (instr(lower(sender), lower(")
            .push_bind(q)
            .push(")) > 0 OR instr(lower(coalesce(summary, '')), lower(")
            .push_bind(q)
            .push(")) > 0)");
    }
    if let Some((at, id)) = cursor {
        sql.push(" AND (sort_at, id) < (")
            .push_bind(at)
            .push(", ")
            .push_bind(id)
            .push(")");
    }
    sql.push(" ORDER BY sort_at DESC, id DESC LIMIT ")
        .push_bind(i64::from(limit) + 1);
    Ok(sql)
}

#[cfg(test)]
mod tests {
    use crate::{
        models::{Environment, ListQuery, MailItem},
        store::{Store, list_query},
    };
    use sqlx::{Execute, QueryBuilder, Row, Sqlite};

    #[tokio::test]
    async fn actual_list_query_uses_keyset_indexes_on_representative_data() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(
            &directory.path().join("mail.sqlite3"),
            Environment::Sandbox,
            Some("example-location"),
        )
        .await
        .unwrap();
        let items = (0..5000)
            .map(|n| MailItem {
                id: format!("example-{n:05}"),
                sender: "Example Sender".into(),
                received_at: None,
                first_seen_at: 100 + n,
                observed_at: 100 + n,
                archived_at: None,
                read_at: None,
                clear_at: None,
                scan_status: if n % 5 == 0 {
                    "processing"
                } else {
                    "completed"
                }
                .into(),
                scan_notice: None,
                shred_status: None,
                summary: None,
                has_envelope: false,
                has_pdf: false,
                has_ocr: false,
                revision: 1,
            })
            .collect::<Vec<_>>();
        for (page, chunk) in items.chunks(500).enumerate() {
            store
                .ingest_page("example-location", chunk, true, None, page == 9)
                .await
                .unwrap();
        }
        sqlx::query("ANALYZE").execute(store.pool()).await.unwrap();
        for (filters, index) in [
            (
                ListQuery {
                    page_token: Some("2500:example-02500".into()),
                    ..Default::default()
                },
                "items_page",
            ),
            (
                ListQuery {
                    status: Some("processing".into()),
                    page_token: Some("2500:example-02500".into()),
                    ..Default::default()
                },
                "items_scan_page",
            ),
        ] {
            let mut builder = list_query(&filters).unwrap();
            let mut query = builder.build();
            let arguments = query.take_arguments().unwrap().unwrap();
            let statement = query.sql();
            let mut explain =
                QueryBuilder::<Sqlite>::with_arguments("EXPLAIN QUERY PLAN ", arguments);
            explain.push(statement.as_str());
            let rows = explain.build().fetch_all(store.pool()).await.unwrap();
            let details = rows
                .iter()
                .map(|row| row.get::<String, _>("detail"))
                .collect::<Vec<_>>();
            println!("{}: {details:?}", statement.as_str());
            assert!(details.iter().any(|detail| detail.contains(index)));
            assert!(!details.iter().any(|detail| detail.contains("TEMP B-TREE")));
            assert_eq!(store.list(&filters).await.unwrap().items.len(), 50);
        }
        store.close().await.unwrap();
    }
}
