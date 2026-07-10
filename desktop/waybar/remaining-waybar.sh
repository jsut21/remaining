#!/usr/bin/env bash
set -euo pipefail

remaining --json --no-color | jq -c '{
  text: (
    .[0].name + " " +
    (if .[0].windows[0].remaining_percent == null
      then "?"
      else ((.[0].windows[0].remaining_percent | round | tostring) + "%")
    end)
  ),
  tooltip: (map(.name) | join("\n"))
}'
