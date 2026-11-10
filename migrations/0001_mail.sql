CREATE TABLE binding (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    instance_id TEXT NOT NULL,
    environment TEXT NOT NULL,
    location_id TEXT NOT NULL
);
CREATE TABLE items (
    id TEXT PRIMARY KEY,
    sender TEXT NOT NULL,
    scan_status TEXT NOT NULL,
    summary TEXT,
    archived INTEGER NOT NULL CHECK (archived IN (0, 1)),
    unread INTEGER NOT NULL CHECK (unread IN (0, 1)),
    clear_at INTEGER,
    body TEXT NOT NULL,
    sort_at INTEGER NOT NULL,
    first_seen_at INTEGER NOT NULL DEFAULT (unixepoch()),
    revision INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX items_page ON items (sort_at DESC, id DESC);
CREATE INDEX items_scan_page ON items (scan_status, sort_at DESC, id DESC);
CREATE INDEX items_deadline ON items (clear_at) WHERE clear_at IS NOT NULL;
CREATE TABLE sync_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    last_success_at INTEGER,
    last_attempt_at INTEGER,
    error_code TEXT,
    initial_sync_complete INTEGER NOT NULL DEFAULT 0,
    archived_phase INTEGER NOT NULL DEFAULT 0,
    cursor TEXT
);
INSERT INTO sync_state (singleton) VALUES (1);
CREATE TABLE operations (
    id TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'completed', 'failed')),
    error_code TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    completed_at INTEGER
);
CREATE UNIQUE INDEX one_pending_sync ON operations ((1)) WHERE state IN ('queued', 'running');
CREATE TABLE events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id TEXT NOT NULL REFERENCES items(id),
    kind TEXT NOT NULL,
    revision INTEGER NOT NULL,
    notification_key TEXT NOT NULL,
    observed_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE (item_id, kind, notification_key)
);

CREATE INDEX events_item_page ON events (item_id, sequence);
