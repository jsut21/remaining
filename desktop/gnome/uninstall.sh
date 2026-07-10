#!/usr/bin/env bash
set -euo pipefail

UUID="remaining@jsut21.github.io"
SCHEMA_ID="io.github.jsut21.remaining"
DEST="${HOME}/.local/share/gnome-shell/extensions/${UUID}"
SCHEMA_DIR="${DEST}/schemas"

remove_extension_from_list() {
  local key="$1" uuid="$2" current next first later only
  current="$(gsettings get org.gnome.shell "${key}")"
  first="'${uuid}', "
  later=", '${uuid}'"
  only="'${uuid}'"
  next="${current//${first}/}"
  next="${next//${later}/}"
  next="${next//${only}/}"
  if [[ "${next}" != "${current}" ]]; then
    gsettings set org.gnome.shell "${key}" "${next}"
  fi
}

if command -v gnome-extensions >/dev/null 2>&1; then
  gnome-extensions disable "${UUID}" >/dev/null 2>&1 || true
fi
if command -v gsettings >/dev/null 2>&1; then
  remove_extension_from_list enabled-extensions "${UUID}"
  remove_extension_from_list disabled-extensions "${UUID}"
fi

if [[ "${1:-}" == "--purge-settings" ]] &&
   command -v gsettings >/dev/null 2>&1 &&
   [[ -f "${SCHEMA_DIR}/gschemas.compiled" ]]; then
  gsettings --schemadir "${SCHEMA_DIR}" reset-recursively "${SCHEMA_ID}"
fi

rm -rf "${DEST}"
echo "Removed ${UUID}. The remaining CLI was not removed."
