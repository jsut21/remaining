# JSON output

Desktop integrations should call an explicit provider set:

```sh
remaining --json --no-color --provider codex,zai,ollama
```

The schema lives at `schemas/remaining.schema.json`.

Important provider fields:

- `provider`: stable provider id.
- `status`: `available`, `auth_required`, `network_error`, or `unavailable`.
- `source`: normalized source type.
- `source_label`: safe label without credentials, endpoint paths, or absolute home paths.
- `windows`: quota windows such as `5h`, `weekly`, and `monthly`.
- `credits`: balances or reset-credit inventory.
- `metrics`: non-quota measurements such as tool units or local cost.

New integrations should prefer `remaining_percent`, `used_units`, `remaining_units`, `limit_units`, and `unit`. Unknown usage is represented by null percentage fields and `usage_known: false`.
