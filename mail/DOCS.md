# Mail setup

Set `stable_api_key` in the app options, then select a location in the dashboard.
The same key can be reused by deployments selecting different locations.
`location_id` can be supplied directly for unattended setup. It must match any
existing database binding.

`environment` is `production` or `sandbox`. `poll_seconds` defaults to 300 and
must be between 60 and 86400. Native API port exposure is disabled by default.

The dashboard requires an HA administrator. The companion integration exposes
counts, connection health, and content-free notifications. Mail text and PDFs
are available only to administrators through the authenticated bridge.

Mail reads existing scans. Stable's published API schema does not establish a
scan-request operation, so physical actions remain in Stable's dashboard.

After reactivation, finish Stable's address authorization before expecting
contents scans. The app reports missing or failed data rather than treating it
as an empty mailbox.
