# Build Kebloop for Linux

Kebloop's Linux app is built with Rust, GTK4, libadwaita, and gtk4-layer-shell. The Linux source is part of the main repository.

## Prerequisites

- Rust 1.92 or later.
- `pkg-config` and development packages for GTK 4.14 or later, libadwaita 1.6 or later, and gtk4-layer-shell.

Install the development packages provided by your distribution. On Arch Linux, the [`kebloop-git` package](../linux/packaging/arch/PKGBUILD) declares these dependencies.

## Install

Clone the repository and run the installer from its root:

```sh
git clone https://github.com/kubamus/kebloop.git
cd kebloop
./linux/scripts/install.sh
```

The installer builds a release binary and installs it with the desktop entry, AppStream metadata, and icons under `~/.local`. Ensure `~/.local/bin` is in your `PATH`. It does not invoke `sudo` or `doas`; for a system-wide install, run it with the appropriate privilege yourself:

```sh
sudo ./linux/scripts/install.sh --prefix /usr/local
```

Other installer options:

```sh
DESTDIR=/tmp/stage ./linux/scripts/install.sh --prefix /usr
./linux/scripts/install.sh --uninstall
```

On Arch Linux, you can build and install a package from the repository:

```sh
cd linux/packaging/arch
makepkg -si
```

## Build and develop

To build without installing, run these commands from the repository root:

```sh
cd linux
cargo build --release --locked
```

The binary is written to `target/release/kebloop`. The installer builds this same release binary by default; pass `--no-build` only after building it yourself.

The Rust workspace has four crates: `kebloop-core` (engines), `kebloop-services` (stores and catalogs), `kebloop-platform` (Linux integration), and `kebloop` (GTK UI and CLI). See the [architecture guide](../linux/ARCHITECTURE.md) for details. Localization catalogs are shared with the macOS app in `../Sources/Kebloop/Resources`.

Run the workspace checks from the repository root:

```sh
cd linux
cargo build --locked
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

For tests, `KEBLOOP_APP_ID` can give a test instance a separate application id, and `KEBLOOP_DISABLE_SERVICES` accepts a comma-separated list of `tray`, `shortcuts`, and `clipboard` to skip starting those services.

Live desktop tests are opt-in and use isolated sessions. The [GNOME Shell integration guide](../linux/integrations/gnome-shell/README.md) covers clipboard, native paste, and window controls; the [KWin guide](../linux/integrations/kwin/README.md) covers native controls and monitors. X11 clipboard, XTEST, and EWMH tests need a private Xvfb display. From the repository root, run:

```sh
cd linux
cargo test -p kebloop-platform private_x11 -- --ignored --test-threads=1
cargo test -p kebloop-platform compositor::x11::tests::live_controls -- --ignored --test-threads=1
tests/gtk_snippets.py target/release/kebloop
cargo test -p kebloop-platform private_selected_text -- --ignored
xvfb-run -a dbus-run-session sh -c 'openbox >/tmp/kebloop-openbox.log 2>&1 & python3 tests/gtk_workflows.py target/release/kebloop'
```

The GTK smoke test also needs private Xvfb/D-Bus, openbox, python3-gi, xdotool, and xclip. The workflow keyboard smoke additionally uses python3-pyatspi for accessible preview/utility checks. The workspace checks are local; this repository does not configure CI. Linux has been verified on Ubuntu 26.04 ARM64 with GNOME Shell 50.1, KWin 6.6.6, and Xvfb/Openbox. Other desktop versions and physical monitor or hardware configurations require their own testing.

Dictionary and dictation dependencies are optional: install `sdcv` with a StarDict dictionary for definitions, and `alsa-utils`/`whisper.cpp` with a local model for on-device dictation. They are selected from keyboard-accessible launcher pages. Calendar commands read explicitly selected `.ics` files. See [the Linux workflow support matrix](../linux/README.md#workflow-commands).
