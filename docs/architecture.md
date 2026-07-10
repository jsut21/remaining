# Architecture

`remaining` has one source of truth: the Rust CLI. Desktop integrations select providers, invoke the CLI, and render its JSON output.

## Layers

- `src/cli.rs`: command line flags, provider selection, and watch mode.
- `src/engine`: provider registry, fetch options, dispatch, and shared snapshot types.
- `src/providers`: provider-specific authentication, requests, local reads, and parsing.
- `src/output`: terminal and JSON serialization.
- `desktop`: thin Linux desktop integrations.

Provider logic should not be copied into GNOME Shell JavaScript, Waybar scripts, or other UI layers.

## Access boundaries

`FetchOptions` carries request timeout and explicit permissions for browser-profile reads and custom endpoints. A provider must not bypass those options. Official HTTPS endpoints are the default trust boundary; environment endpoint overrides do not imply permission to send credentials elsewhere.

## Snapshot contract

Quota usage is optional. `used_percent: null` means the source did not expose trustworthy usage. Consumers use `usage_known` and `is_synthetic` instead of turning missing data into zero usage.

The JSON array shape is retained for desktop compatibility. Every provider object has `schema_version: 2`.

