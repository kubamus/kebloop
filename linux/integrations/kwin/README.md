# Native KWin window controls

On Plasma 6 Wayland, Kebloop uses KWin's built-in scripting API for window
listing, focus, minimize/show, close, monitor workareas and frame placement.
No KWin plugin installation or `qdbus` executable is required. Plasma X11
uses the EWMH backend instead.

Each request loads a private temporary script through `org.kde.KWin`, sends
its result back to a unique session-bus connection, then unloads the script.
The receiver accepts replies only from the KWin process that loaded it.
D-Bus methods and result waits have two-second timeouts. The script file is
removed automatically. The scripting interface must be available on the
user's session bus; this is unavailable inside sandboxes that deny KWin access.

The script targets the [KWin 6 scripting API](https://develop.kde.org/docs/plasma/kwin/api/).
Arrange/Restore saves and restores window geometry. Frame placement removes
maximization and KWin tile association; restoring a custom KWin tile association
is not currently supported.

`node --test linux/integrations/kwin/request.test.mjs` checks the script's
behavior against API fixtures. The Rust ignored test
`compositor::kwin::tests::live_read_only` requires a real KWin 6 session.
`compositor::kwin::tests::live_controls` creates an isolated XWayland test
window and checks focus, minimize/show, frame placement, crossing outputs,
and close delivery. Both live tests passed with KWin 6.6.6 in a private
headless session with two virtual outputs.
