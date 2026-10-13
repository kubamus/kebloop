# Optional GNOME Shell integration

Install as your desktop user, from this directory:

```sh
mkdir -p ~/.local/share/gnome-shell/extensions/kebloop@kebloop.app
cp kebloop@kebloop.app/{extension.js,metadata.json} ~/.local/share/gnome-shell/extensions/kebloop@kebloop.app/
gnome-extensions enable kebloop@kebloop.app
```

Log out and back in after first installation on Wayland. Disable with
`gnome-extensions disable kebloop@kebloop.app`.

This opt-in extension supplies window enumeration, focus, close, minimize and
exact geometry and native paste shortcuts on GNOME Wayland without root or enabling Shell Eval. The session
bus authenticates clients as the desktop user. Other processes running as that
user can access this bridge, including the clipboard. No network listener exists.

`org.kebloop.Gnome1` at `/org/kebloop/Gnome` has version 1. GetClipboard returns
metadata JSON with revision, offered mimes, readiness and source_app;
GetClipboardData separately returns cached multi-format base64 data. Owner changes and clears
update revision immediately. Reads are capped at 8 MiB and cancelled after two
seconds. Text, HTML, RTF, PNG and URI lists are supported; primary selection is ignored.
Reads occur only while the watcher enables recording; sensitive offered MIME
markers suppress data reads. Each selected MIME is capped at 8 MiB. Writes through
Mutter memory sources advertise one MIME (plain text is preferred for mixed
payloads), so rich/plain multi-format write fidelity is limited. Native paste uses
Mutter virtual keyboard events, without external typing tools.
Source app is always null: GNOME does not expose trustworthy clipboard source
identity, so focused application is never substituted. Windows are represented
as floating; monitor names are indices within the current monitor configuration.

Targets Shell 45–51. The integration smoke test passed on GNOME Shell 50.1 /
Mutter 50.1 in Ubuntu 26.04, using a private session bus, headless Wayland,
software rendering, and a real GTK4 entry. It verifies window focus, exact frame,
minimize/show/close, monitor usable area, external text history, native paste into
the entry, own text and PNG byte round trips, pause/resume, sensitive markers,
and immediate revision changes on clearing. Other Shell releases and physical
multi-monitor sessions still need testing.

Run the tests from this directory:

```sh
node test.mjs
# Linux dependencies: gnome-shell, gjs, wl-clipboard, python3, GTK4 introspection
sh tests/run.sh
```

The smoke test installs the extension only into temporary XDG directories;
it does not change the current desktop's extension settings or clipboard.

References: [Meta.Window](https://gnome.pages.gitlab.gnome.org/mutter/meta/class.Window.html),
[Meta.Selection](https://gnome.pages.gitlab.gnome.org/mutter/meta/class.Selection.html),
[GJS D-Bus](https://gjs.guide/guides/gio/dbus.html).
