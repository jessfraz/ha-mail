# Mail

A physical mailbox for Home Assistant, backed by Stable.

Mail shows envelopes, scan PDFs, OCR, summaries, and storage deadlines. It keeps
a local SQLite record and polls Stable for changes. Forwarding, shredding,
deposits, and scan requests stay in Stable.

## Install

Add this repository to the Home Assistant app store, install Mail, and set the
Stable API key in the app options. Open the dashboard and choose the location
for this deployment. A shared Stable credential is fine; each database is
bound to one location and environment.

Sync starts automatically once the key and location are ready, then repeats
every five minutes. Change `poll_seconds` in the app options to use another
interval. Sync continues when the dashboard is closed.

Install the companion integration from this repository through Marketplace or
HACS, then add **Mail** in Devices & services. Supervisor discovery pairs it
without copying tokens. The integration exposes counts and sync health, not
mail contents. Choose which count entities to expose to Assist.

Location selection is permanent for a database. To move to another mailbox,
create a fresh deployment. Do not reuse another location's database.

## CLI

```sh
ha-mail --help
ha-mail health
ha-mail items list --unread
ha-mail items get ITEM_ID
ha-mail items pdf ITEM_ID --output mail-scan.pdf
ha-mail sync request --idempotency-key REQUEST_ID
```

Native access uses `MAIL_API_URL` and `MAIL_API_TOKEN`. Use
`--transport home-assistant` with `HOME_ASSISTANT_URL` and
`HOME_ASSISTANT_TOKEN` for the HA
bridge. There is no credential or property fallback. PDF files are written
owner-only and never overwrite an existing file.

The CLI is also available as `nix run github:jessfraz/ha-mail#ha-mail`.

## Run locally

```sh
nix develop
export MAIL_STABLE_KEY=...
export MAIL_API_TOKEN=... # at least 32 printable characters
cargo run --bin mail
```

The native listener binds to loopback by default. `MAIL_ENVIRONMENT=sandbox`
uses Stable's separate sandbox. Never use production mail as test fixtures.

Run `uv run --frozen python scripts/build-integration` before the Home Assistant
tests. Add `--output dist` to build the Marketplace ZIP from indexed source.
Rust owns storage and API types. API docs are served at `/openapi.json`.

## Backups

The app uses cold native HA backups. Preserve the complete private data
directory, including `credentials.json` and `mail.sqlite3`. A restored database
retains its location binding, records, and sync checkpoints. Stable is still
the authority for physical handling and provider state.
