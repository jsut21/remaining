# Provider authoring

1. Add a `ProviderId` variant in `src/engine/snapshot.rs`.
2. Add metadata in `src/engine/registry.rs`.
3. Add a module under `src/providers/<provider>.rs`.
4. Return a `ProviderResult` with quota windows, credits, or metrics.
5. Add dispatch in `fetch_provider`.
6. Add parser fixtures and document credentials and reset windows.
7. Add the provider to the GNOME settings catalog when desktop support is intended.

Rules:

- Keep credentials provider-local.
- Do not print tokens, cookies, account identifiers, absolute home paths, or endpoint queries.
- Bound every network request with `FetchOptions::timeout`.
- Read browser profiles only when `allow_browser_cookies` is true.
- Restrict credential-bearing requests to official HTTPS domains unless `allow_custom_endpoints` is true.
- Reject plain HTTP and URL-embedded credentials even when custom endpoints are enabled.
- Prefer `auth_required` or unknown usage over a synthetic 100% remaining value.
