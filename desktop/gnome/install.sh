#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UUID="remaining@jsut21.github.io"
LEGACY_UUID="remainingbar@local"
SCHEMA_ID="io.github.jsut21.remaining"
LEGACY_SCHEMA_ID="org.gnome.shell.extensions.remainingbar"
SOURCE="${SCRIPT_DIR}/${UUID}"
DEST="${HOME}/.local/share/gnome-shell/extensions/${UUID}"
LEGACY_DEST="${HOME}/.local/share/gnome-shell/extensions/${LEGACY_UUID}"
SCHEMA_DEST="${DEST}/schemas"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 is required to install ${UUID}." >&2
    exit 1
  fi
}

queue_extension_enable() {
  local enabled next needle
  enabled="$(gsettings get org.gnome.shell enabled-extensions)"
  needle="'${UUID}'"
  if [[ "${enabled}" == *"${needle}"* ]]; then
    return
  fi

  case "${enabled}" in
    "@as []"|"[]") next="['${UUID}']" ;;
    *)
      next="${enabled%]}"
      next="${next}, '${UUID}']"
      ;;
  esac
  gsettings set org.gnome.shell enabled-extensions "${next}"
}

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

require_command remaining
require_command gnome-extensions
require_command gsettings
require_command glib-compile-schemas

if ! remaining --version 2>/dev/null | grep -Eq '^remaining [0-9]'; then
  echo "The remaining CLI is missing or incompatible. Run: cargo install --locked --path ." >&2
  exit 1
fi

if [[ ! -d "${SOURCE}" ]]; then
  echo "Extension source not found at ${SOURCE}." >&2
  exit 1
fi

legacy_panel=""
if [[ -f "${LEGACY_DEST}/schemas/gschemas.compiled" ]]; then
  legacy_panel="$(gsettings --schemadir "${LEGACY_DEST}/schemas" get "${LEGACY_SCHEMA_ID}" panel-provider 2>/dev/null || true)"
  legacy_panel="${legacy_panel#\'}"
  legacy_panel="${legacy_panel%\'}"
fi

if gnome-extensions info "${LEGACY_UUID}" >/dev/null 2>&1; then
  gnome-extensions disable "${LEGACY_UUID}" >/dev/null 2>&1 || true
fi
remove_extension_from_list enabled-extensions "${LEGACY_UUID}"

rm -rf "${DEST}"
mkdir -p "${DEST}"
cp -a "${SOURCE}/." "${DEST}/"
glib-compile-schemas --strict "${SCHEMA_DEST}"

case "${legacy_panel}" in
  codex|zai|ollama|opencode-go|minimax)
    gsettings --schemadir "${SCHEMA_DEST}" set "${SCHEMA_ID}" panel-provider "${legacy_panel}"
    ;;
esac

gsettings set org.gnome.shell disable-user-extensions false
remove_extension_from_list disabled-extensions "${LEGACY_UUID}"
remove_extension_from_list disabled-extensions "${UUID}"
queue_extension_enable

if [[ -d "${LEGACY_DEST}" ]]; then
  rm -rf "${LEGACY_DEST}"
fi

echo "Installed ${UUID} and added it to GNOME's enabled list."
echo "Log out and back in to load new extension code safely."
