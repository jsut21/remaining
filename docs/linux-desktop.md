# Linux desktop integrations

The CLI JSON is the integration contract. UIs should not fetch provider data directly.

## GNOME

The supported integration is under `desktop/gnome` and uses UUID `remaining@jsut21.github.io`. Its popup controls provider selection and the two sensitive access permissions passed to the CLI.

## Waybar

`desktop/waybar` contains an experimental custom-module script. It requires `jq` and has no shared settings with the GNOME extension.

## AppIndicator

No AppIndicator implementation is shipped yet. The directory records a possible fallback for desktops without GNOME Shell extensions.
