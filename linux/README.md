# Kebloop for Linux

A native Linux port of Kebloop: one search field for apps, files, calculations, clipboard history, quicklinks, snippets, and emoji. Built with Rust, GTK4, libadwaita, and gtk4-layer-shell. It runs locally and needs no account.

The behaviour mirrors the macOS app; see the [usage guide](../docs/usage.md) for commands, calculator syntax, and conversions.

## Requirements

- **Runtime:** GTK 4.14 or later, libadwaita 1.6 or later, and gtk4-layer-shell.
- **Build:** Rust 1.92 or later, `pkg-config`, and the development headers of the libraries above.
- **Optional:** `wtype` (wlroots compositors, niri) or `ydotool` for direct paste into the previous window; `wpctl` or `pactl` for Toggle Mute; `gsettings` (or the desktop appearance portal) for Toggle Appearance; `loginctl` (or logind over D-Bus) for Lock Screen, Sleep, Restart and Log Out.

| Session | Launcher | Clipboard history |
| --- | --- | --- |
| niri, Sway, Hyprland, KDE Plasma (Wayland) | Layer-shell overlay, top-centered | Yes |
| GNOME (Wayland) | Regular fallback window | Yes with the optional GNOME Shell extension |
| X11 | Regular fallback window | Yes, through XFixes and X11 selections |

On every session, `kebloop toggle` works from any compositor keybinding. Where the GlobalShortcuts portal exists (KDE, GNOME 48+, Hyprland), Kebloop also registers its shortcuts there.

## Build and install

```sh
git clone https://github.com/kubamus/kebloop.git
cd kebloop
git checkout linux-rust
linux/scripts/install.sh
```

The script builds a release binary and installs it, the desktop entry, the AppStream metadata, and the icon under `~/.local`. Make sure `~/.local/bin` is in your `PATH`. Options:

```sh
linux/scripts/install.sh --prefix /usr/local   # run with doas/sudo yourself
DESTDIR=/tmp/stage linux/scripts/install.sh --prefix /usr
linux/scripts/install.sh --uninstall
```

On Arch Linux, build a package from the repository instead:

```sh
cd linux/packaging/arch
makepkg -si
```

## Bind the launcher shortcut

Run `kebloop --background` once (or enable launch at login), then bind a key to `kebloop toggle`.

niri (`config.kdl`):

```kdl
binds {
    Alt+Space { spawn "kebloop" "toggle"; }
}
```

Hyprland (`hyprland.conf`):

```ini
bind = ALT, SPACE, exec, kebloop toggle
```

Sway (`config`):

```
bindsym Mod1+space exec kebloop toggle
```

i3 (`config`):

```
bindsym Mod1+space exec --no-startup-id kebloop toggle
```

KDE Plasma and GNOME: either accept the global-shortcut request from Kebloop, or add a custom shortcut in System Settings that runs `kebloop toggle`. Settings also shows ready-to-paste snippets, including `kebloop command <id>` and `kebloop launch <desktop-id>` bindings for individual commands and apps.

## Optional desktop integration

GNOME Wayland clipboard history, window controls and native paste use the bundled [GNOME Shell extension](integrations/gnome-shell/README.md). It is installed and enabled explicitly by the user. Extension files are also installed under `<prefix>/share/kebloop/integrations/gnome-shell/`. GNOME clipboard history reads multiple formats, but publishing through the extension advertises one MIME format, so copying mixed rich/plain payloads has limited fidelity.

Plasma 6 Wayland uses [KWin’s native scripting API](integrations/kwin/README.md) for window controls; no plugin is required. Neither bridge requires root or Shell Eval. X11 integrations use native EWMH, XFixes and XTEST. Compositor-specific behavior remains on niri.

## Autostart

Enable **Launch at login** in Settings or in the welcome walkthrough. This writes `~/.config/autostart/io.github.kubamus.Kebloop.desktop`, which runs `kebloop --background` (using the absolute path of the binary unless it is installed in a standard system directory such as `/usr/bin`). Closing the launcher leaves Kebloop running; quit it from the tray menu (when your desktop has a StatusNotifier tray) or with `kebloop quit`.

## Command line

| Command | Action |
| --- | --- |
| `kebloop` | Start and show the launcher |
| `kebloop --background` | Start without showing anything |
| `kebloop toggle` / `show` / `hide` | Control the launcher in the running instance |
| `kebloop settings [page]` | Open Settings, optionally on a page such as `general` or `clipboard` |
| `kebloop command <id>` | Run `emoji`, `clipboard`, `snippet:<id>`, `script:<id>`, `extension:<id>`, or `link:<id>` |
| `kebloop launch <desktop-id>` | Launch an app, for example `org.mozilla.firefox.desktop` |
| `kebloop quit` | Quit the running instance |

Kebloop is single-instance (D-Bus name `io.github.kubamus.Kebloop`); these commands are forwarded to the running copy.

## Storage

| Data | Location |
| --- | --- |
| Settings and profiles | `$XDG_CONFIG_HOME/kebloop/config.json` |
| Clipboard history and attachments | `$XDG_DATA_HOME/kebloop/` |
| Calculator history (local, never exported) | `$XDG_DATA_HOME/kebloop/calculation-history.json` |
| Application index cache | `$XDG_CACHE_HOME/kebloop/` |
| Saved currency exchange rates | `$XDG_CACHE_HOME/kebloop/currency-rates.json` |

Defaults are `~/.config`, `~/.local/share`, and `~/.cache`. Configuration exports use the same portable format as macOS. A symlinked `config.json` is followed, not replaced.

Usage counters and the clipboard history are written in the background shortly after a change and flushed when Kebloop quits (including on `SIGTERM`). If the configuration or clipboard history cannot be opened, Kebloop still starts (with default settings, or without history) and explains why under **Help → Startup notices** and in a desktop notification; an unreadable file is moved aside as `*.corrupt` and its path is shown there.

## Differences from macOS

- No Apple Shortcuts and no Siri handoff. AI handoff opens ChatGPT or Claude in your default browser.
- Scripts and extensions run with a cleared environment containing `PATH`, `LANG` and the session variables `HOME`, `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, `DISPLAY`, `DBUS_SESSION_BUS_ADDRESS` and `XDG_CURRENT_DESKTOP`, so tools such as `notify-send`, `wl-copy`, or `playerctl` can reach your session. The macOS app passes only `PATH` and `LANG`. No other variable, and in particular no token or credential, is inherited.
- Clipboard history reads no clipboard content while recording is off or paused, and drops text over 100 KB (a bounded rich-text or image payload from the same copy is still kept, as on macOS).
- File search honours `.gitignore` files only inside git repositories. The index is rebuilt every 30 minutes; if the entry cap truncates it, the same files are kept every time.
- The launcher holds the keyboard exclusively while it is shown, and releases that hold while a confirmation dialog or the action menu is open.
- Direct paste (the macOS Accessibility paste of Clipboard, Emoji and snippet Insert) types Ctrl+V (Ctrl+Shift+V in known terminals) into the window that was focused before the launcher. X11 uses native XTEST; GNOME uses its optional extension. Other Wayland sessions use `wtype` or `ydotool`. It is on by default when one of them is installed and is switched in Settings → General → Direct Paste (stored as `directPasteEnabled`; the old `snippetPasteEnabled` is still read). Without the tool, Kebloop copies instead and says so ("install wtype"). Paste is cancelled if the window or the clipboard changed meanwhile.
- Clipboard source-app exclusions work on X11 when the selection owner exposes a class or process identity. Enter desktop-file IDs or owner identities in Settings → Clipboard. Proxy owners without an identifiable source are unknown. Native Wayland and the GNOME extension cannot reliably identify the original source application; Kebloop never substitutes the foreground app. Sensitive markers are still skipped.
- Snippet keyword expansion is opt-in in Settings → Snippets. It uses AT-SPI accessibility text events on X11 and Wayland. It expands a keyword followed by a space in focused, editable text fields; password fields and selected text are skipped. No global keyboard hook or privileged input device is used. Applications that do not expose accessible editable text (including many terminals) require the Snippets command. The accessibility bus must be enabled in your desktop settings.
- System commands (Lock Screen, Sleep, Restart, Log Out, Toggle Mute, Toggle Appearance) use logind (`loginctl` as fallback), `wpctl`/`pactl` and the appearance portal/`gsettings`. A command is listed only when its backend works. Restart and Log Out ask for confirmation in Kebloop (macOS shows the system dialog). Set `KEBLOOP_DRY_RUN_SYSTEM_ACTIONS=<file>` to log the actions instead of running them (for testing).
- Window-management commands (halves, quarters, thirds, maximize, center, restore, displays) are offered on niri, Sway, i3 and Hyprland and act on the window focused before the launcher opened. Sway, i3 and Hyprland match macOS. niri has no fixed screen regions: halves, thirds and maximize set the column width (shown as "Approximate" in the row subtitle), quarters and top/bottom halves need a floating window, and Restore only restores the size. GNOME supports them with the optional extension; Plasma 6 Wayland uses native KWin scripting and X11 uses EWMH. Restore preserves geometry but does not restore custom KWin tile associations. Settings → Window Management records one shortcut per action; where the portal is unavailable it shows `kebloop command <id>` snippets for the compositor.
- Running applications show "Running" and offer Hide, Quit, Force Quit (with confirmation) and Restart where the compositor allows it (niri cannot hide windows, so Hide is not offered there). Choosing a running app with an assigned app shortcut toggles it like on macOS: focus, then hide where possible.
- Snippet keywords can be edited in Settings; a blank field clears the keyword. Automatic expansion only applies to snippets whose placeholders have defaults, and replacement is limited to 4,096 characters.
- Currency, time-zone and relative-date calculator results work as on macOS; exchange rates refresh in the background and results older than 24 hours are marked "cached". Copying a calculator result records it in Calculator History (command in the root search).
- Pinned apps show a pin glyph in results and are announced as pinned to screen readers.
- Named time zones (`time in Tokyo`) use the system `tzdata`.
- Apps come from XDG `.desktop` files (including Flatpak and Snap exports); file search indexes the folders chosen in Settings.
- Shutdown asks for confirmation in Kebloop, then uses logind.

## Privacy

Same as the macOS app. Core features run locally. The one exception is currency conversion: when a query names a currency pair (for example `100 USD to EUR`) and no rate for it was downloaded in the last 24 hours, Kebloop requests that pair from [Frankfurter](https://frankfurter.dev/) (`https://api.frankfurter.dev/v2/rate/usd/eur`). Only the two currency codes are sent, never the rest of your query; rates are cached for offline use for up to 30 days. AI handoff sends only your query to the provider you choose, in your browser; no API key is required. Sensitive clipboard types are not recorded, and clipboard files are stored with owner-only permissions.

## Development

```sh
cd linux
cargo build
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The workspace has four crates: `kebloop-core` (engines), `kebloop-services` (stores and catalogs), `kebloop-platform` (Linux integration), and `kebloop` (GTK UI and CLI). See [ARCHITECTURE.md](ARCHITECTURE.md) for details and the macOS-to-Linux mapping. Localization catalogs are shared with the macOS app from `Sources/Kebloop/Resources`.

Environment variables for testing:

- `KEBLOOP_APP_ID`: use another application id so a test instance does not collide with your running one.
- `KEBLOOP_DISABLE_SERVICES`: comma-separated list of `tray`, `shortcuts`, `clipboard` to leave unstarted.

## Validation

Run the full Rust checks above locally; this repository does not configure CI. Live desktop tests are opt-in and use isolated sessions:

- [GNOME Shell smoke tests](integrations/gnome-shell/README.md) cover clipboard, native paste and window controls.
- [KWin tests](integrations/kwin/README.md) cover native controls and monitors.
- X11 clipboard, XTEST and EWMH live tests require a private Xvfb display (`cargo test -p kebloop-platform private_x11 -- --ignored --test-threads=1` and `compositor::x11::tests::live_controls`).
- `tests/gtk_snippets.py target/release/kebloop` verifies real GTK snippet expansion and clipboard rendering under private Xvfb/D-Bus with openbox, python3-gi, xdotool and xclip.

Verified on Ubuntu 26.04 ARM64, GNOME Shell 50.1, KWin 6.6.6, and Xvfb/Openbox. Other desktop versions and physical monitor/hardware configurations require their own testing.

## Third-party data

- The emoji picker bundles Unicode Emoji 18.0 (`emoji-test.txt`, <https://unicode.org/Public/18.0.0/emoji/emoji-test.txt>). The data is licensed under the [Unicode License V3](../Sources/KebloopCore/Resources/unicode-license.txt), which the package installs next to the MIT licence; the same text is available to the application as `kebloop_core::EmojiCatalog::UNICODE_LICENSE`. See also [docs/unicode-emoji-attribution.md](../docs/unicode-emoji-attribution.md).
- Exchange rates come from [Frankfurter](https://frankfurter.dev/), which publishes daily mid-market reference rates.
- The symbolic fallback icons in `data/icons/hicolor/symbolic/apps` are copied from the GNOME Adwaita icon theme (CC-BY-SA-3.0 or LGPL-3.0), see [data/icons/README.md](data/icons/README.md). They are installed under `share/kebloop/icons` and only used when the active icon theme lacks an icon.

## License

MIT, see [LICENSE](../LICENSE).
