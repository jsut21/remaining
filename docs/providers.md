# Providers

Provider extraction is best effort because several sources are undocumented or intended for first-party clients. Only providers selected on the CLI or in GNOME are queried.

## Codex

Reads `~/.codex/auth.json`, refreshes OAuth credentials after eight days when a refresh token exists, and calls the ChatGPT usage endpoint. Reports 5-hour, weekly, named model limits, and reset-credit inventory when present.

The auth file can be updated during token refresh. Replacement is staged with user-only permissions.

## Z.ai

Credential priority:

- `ZAI_API_KEY`, `Z_AI_API_KEY`, or `ZHIPU_API_KEY`
- `~/.api-zai`
- the Z.ai entry in `~/.local/share/opencode/auth.json`

Reports token and tools windows returned by the quota API. Unit counts are derived from `usage`, `currentValue`, and `remaining` when possible; missing values remain unknown. Tool-level `usageDetails` are exposed as metrics.

Endpoint and team overrides include `Z_AI_API_HOST`, `ZAI_API_HOST`, `Z_AI_QUOTA_URL`, `ZAI_QUOTA_URL`, and the organization/project variables in the source.

By default, credentials are sent only to HTTPS hosts under `z.ai`, `bigmodel.cn`, or `zhipuai.cn`. A non-official HTTPS override requires `--allow-custom-endpoints`. Plain HTTP and URLs containing credentials are always rejected.

## Ollama Cloud

Credential priority:

- `OLLAMA_COOKIE_HEADER` or `OLLAMA_SESSION_COOKIE`
- `~/.config/ollama-usage/session`
- Firefox cookies for `ollama.com`, unless `--no-browser-cookies` is used

Chunked session cookies are sent together. The provider parses plan, session or hourly usage, weekly usage, and reset times. Only a response labeled `Session usage` is mapped to a 5-hour window; an hourly response remains an unspecified-duration lane.

## OpenCode Go

Remote sources are attempted before the local fallback:

1. Workspace quota API, with one POST fallback.
2. Workspace page at `https://opencode.ai/workspace/{workspace}/go`.
3. Local SQLite history at `~/.local/share/opencode/opencode.db`.

Workspace and cookie inputs include environment variables and `~/.config/opencode-quota/opencode-go.json`. Chrome cookie decryption is attempted unless `--no-browser-cookies` is used.

The local reader counts only assistant messages whose `providerID` is `opencode-go`. It maps cost to estimate windows based on the service's current limits; these windows are marked synthetic and are not presented as provider-reported quota.

## MiniMax

Credential priority includes MiniMax API key environment variables, `~/.api-minimax`, and the MiniMax entry in OpenCode auth. Manual cookie headers can be supplied with `MINIMAX_COOKIE_HEADER` or `MINIMAX_COOKIE`. Firefox profile scanning is skipped when `--no-browser-cookies` is used.

The provider tries the current Token Plan API, the legacy Coding Plan API, and the China API after a global credential rejection. Endpoint overrides include `MINIMAX_CODING_PLAN_URL`, `MINIMAX_WEB_REMAINS_URL`, `MINIMAX_REMAINS_URL`, and `MINIMAX_HOST`.

By default, credentials are sent only to HTTPS hosts under `minimax.io` or `minimaxi.com`. Other HTTPS endpoints require `--allow-custom-endpoints`; plain HTTP and URLs containing credentials are always rejected.
