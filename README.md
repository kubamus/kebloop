<p align="center">
  <img src="assets/icon.svg" width="80" height="80" alt="Kebloop icon">
</p>

<h1 align="center">Kebloop</h1>

<p align="center">A native macOS launcher for the things you do every day.</p>

Open apps, find files, run calculations, and reach your clipboard, quicklinks, and emoji from one search field. Built with Swift, AppKit, and SwiftUI, Kebloop lives in the menu bar and opens with **Option-Space**.

## Features

- **App search** with fuzzy matching, aliases, and custom app shortcuts.
- **File search** powered by Spotlight, with Git ignore rules respected.
- **Calculator** with functions, percentages, mixed units, durations, named time zones, cached currency conversions, and accepted-result history.
- **Clipboard history** for text, images, and rich text, with pins, retention controls, sensitive-content exclusions, and direct or plain-text paste.
- **Quicklinks** for websites, files, and folders.
- **Emoji picker** with search, categories, and recents.
- **App controls** with running indicators, hide, quit, confirmed force quit, restart, and focus/hide app shortcuts.
- **System actions** for shutdown, lock, sleep, restart, log out, mute, and appearance.
- **Window management** for halves, quarters, thirds, two thirds, maximize, center, restore, and moving between displays, with a dedicated shortcut settings page.
- **Snippets** with prompted/default values, date/time/clipboard variables, and optional local keyword expansion.
- **AI handoff** to ChatGPT, Claude, or Siri using your existing apps and accounts.
- **Native appearance** with Liquid Glass on macOS 26+ and popover material on older releases.

Core features run locally without a Kebloop account. Currency conversion fetches the requested currency pair from Frankfurter and caches rates for offline use. AI handoff sends the query to your selected provider; no API key is needed.

## Build and install

**Runtime:** macOS 14 or later. **Build tools:** Xcode 27 or later, including the macOS 27 SDK. The newer SDK is required to compile the interactive glass API; the resulting app still supports macOS 14+.

Select your Xcode toolchain if needed:

```sh
sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
```

Clone the repository and install:

```sh
git clone https://github.com/kubamus/kebloop.git
cd kebloop
./scripts/install-app.sh
```

The installer builds a release bundle, verifies its signature, installs it at `/Applications/Kebloop.app`, and opens the installed copy. Run the same command after pulling updates. Your preferences and clipboard history are stored outside the app bundle.

If `/Applications` is not writable, install for your user:

```sh
./scripts/install-app.sh --destination "$HOME/Applications"
```

Use `--no-launch` to install without opening the app. To build without installing:

```sh
./scripts/build-app.sh
open dist/Kebloop.app
```

Builds target your Mac's architecture. The script uses an available Apple Development signing identity, falling back to ad-hoc signing. To choose an identity explicitly:

```sh
KEBLOOP_SIGNING_IDENTITY="Apple Development: Your Name (TEAMID)" ./scripts/install-app.sh
```

These are local development builds, not notarized distribution releases. A stable development signature helps macOS retain Siri automation permissions across rebuilds; ad-hoc builds may require granting them again.

## Linux

A native Linux port (Rust, GTK4, libadwaita, Wayland-first) lives in [`linux/`](linux/README.md) on the `linux-rust` branch. It covers app, file, calculator, clipboard, quicklink, snippet, and emoji search, and is bound to a compositor key with `kebloop toggle`. See the [Linux guide](linux/README.md) for requirements, installation, and shortcut setup.

## Getting started

| Action | Shortcut |
| --- | --- |
| Open or hide the launcher | Option-Space |
| Select a result | Up / Down |
| Open a result or copy a calculation | Return |
| Close the launcher or return from a command | Escape |
| Open settings | Command-comma |
| Show selected result actions | Command-K |
| Paste selected clipboard item / emoji | Return |
| Copy selected clipboard item / emoji | Command-Return |
| Paste clipboard item as plain text | Command-Shift-Return |
| Send a query to your AI provider | Tab |

Type **Clipboard History**, **Quicklinks**, **Emoji & Symbols**, or **Open Settings** to open a command. You can change the launcher shortcut in Settings.

Type **shutdown** (or **power off** / **turn off**) and press Return to open the macOS shutdown confirmation. Confirm there to turn off your Mac, or cancel to keep working.

On first launch, a skippable walkthrough lets you choose launch at login and clipboard recording. Manage them later in Settings; existing choices are preserved. **Command-Q hides the interface**; use **Quit Kebloop** in the menu-bar menu to exit.

File results depend on Spotlight indexing and macOS access permissions. Siri handoff requires Siri, Type to Siri, and the system permission described in the [usage guide](docs/usage.md#ai-handoff).

## Development

Open `Package.swift` in Xcode, or use the command line:

```sh
swift build
swift test -c release
```

- [Usage guide](docs/usage.md): commands, calculator syntax, privacy, and storage.
- [Performance profiling](docs/performance.md): repeatable latency, CPU, and memory measurements.
- [Project plan](PLAN.md): scope and acceptance targets.
- [Feature roadmap](docs/roadmap.md): next features, implementation chunks, dependencies, and acceptance criteria.

Milestones 3–4 add clipboard retention/privacy/pins and image/rich-text history, reusable snippets, unit/date calculations, English/Polish UI resources, reviewed local script commands, and a versioned subprocess extension API. See [usage](docs/usage.md), [conversion syntax](docs/conversions.md), [automation examples/API](docs/automation.md), and [validation status](docs/roadmap.md).

See the [feature-by-feature verification report](docs/feature-verification.md) for test coverage and remaining native UI checks.

## License

Kebloop is released under the [MIT License](LICENSE). The bundled Unicode emoji data is covered by the [Unicode License](Sources/KebloopCore/Resources/unicode-license.txt).
