# Changelog

## Unreleased

- Added JSON snapshot schema v2 with explicit unknown-usage semantics.
- Added provider parsers and regression fixtures for five providers.
- Added Codex OAuth refresh, named-limit deduplication, and reset-credit inventory.
- Improved Z.ai unit-derived percentages and tool metrics.
- Distinguished Ollama hourly usage from 5-hour sessions.
- Filtered OpenCode local history by provider and marked estimated limits synthetic.
- Added current and legacy MiniMax plan fallbacks.
- Enabled supported browser-cookie reads for selected providers by default, with `--no-browser-cookies` as an explicit opt-out.
- Kept custom endpoints opt-in and restricted credential-bearing overrides to official HTTPS domains by default.
- Rejected unknown providers and zero-second watch intervals.
- Removed absolute local paths and endpoint queries from source labels.
- Added GNOME provider/privacy settings and migrated to `remaining@jsut21.github.io`.
- Deferred closed-menu rendering, added a subprocess watchdog, fixed enabled/disabled extension-state cleanup, and stopped live-reloading extension code during installation.
- Added GNOME install, migration, and uninstall scripts.
- Added CI, dependency updates, RustSec auditing, package metadata, and public documentation.
