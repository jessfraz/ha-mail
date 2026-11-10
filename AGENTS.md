# Mail

Use `nix develop` and the checks in `.github/workflows/ci.yml`. Keep changes
focused.

Rust owns Stable access, SQLite, and API/domain types. The HA integration is a
thin adapter. Never put credentials, addresses, mail contents, signed document
URLs, or household details in source, logs, diagnostics, or fixtures.

Each database belongs to one environment and Stable location. Validate every
provider item's location before storing it. Document requests must start from
an existing local item. Never turn the service into an arbitrary URL proxy.

No physical-mail mutations are implemented. Do not infer shipped/delivered,
paid, scanned, or shredded outcomes from a request or an incomplete response.

Keep the README concrete. No marketing copy or generic boilerplate headings.
