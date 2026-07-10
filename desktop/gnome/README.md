# GNOME panel extension

The extension calls the `remaining` CLI and renders its JSON output in the GNOME top panel. Provider authentication, network requests, and parsing stay in Rust.

## Requirements

- GNOME Shell 46
- `remaining` available from `PATH`, `~/.local/bin`, or `~/.cargo/bin`
- `gnome-extensions`, `gsettings`, and `glib-compile-schemas`

## Install

```sh
cargo install --locked --path .
./desktop/gnome/install.sh
```

The extension is installed as `remaining@jsut21.github.io`. The installer disables and removes the earlier local-development UUID `remainingbar@local`, preserves its pinned panel provider when possible, and queues the new UUID in GNOME's enabled-extension list.

Use the settings button in the popup to:

- enable or disable providers;
- allow supported browser-cookie reads;
- allow non-official HTTPS endpoints.

Browser cookies are enabled by default and can be disabled in the popup. Custom endpoints remain disabled by default. At least one provider remains enabled.

GNOME 46 caches loaded extension modules for a login session. Log out and back in after installation or an upgrade. The installer does not live-reload the extension because repeated in-session Shell restarts can leave stale module and actor state.

## Remove

```sh
./desktop/gnome/uninstall.sh
```

Add `--purge-settings` to reset the extension settings. The CLI is not removed.
