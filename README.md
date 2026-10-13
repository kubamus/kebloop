<p align="center">
  <img src="assets/icon.svg" width="80" height="80" alt="Kebloop icon">
</p>

<h1 align="center">Kebloop</h1>

<p align="center">A native multiplatform launcher for macOS and Linux.</p>

Open apps, find files, run calculations, and reach your clipboard, quicklinks, snippets, and emoji from one search field. Kebloop uses Swift, AppKit, and SwiftUI on macOS, and Rust, GTK4, and libadwaita on Linux.

## Features

- App and file search with fuzzy matching, aliases, and shortcuts.
- Calculator with unit, currency, date, and time-zone conversions.
- Clipboard history for text, images, and rich text, with pins and privacy controls.
- Quicklinks, reusable snippets, and a searchable emoji picker.
- App controls, system actions, and window management, depending on your desktop.
- AI handoff to ChatGPT or Claude, plus Siri on macOS.
- macOS window/menu search, meeting commands, selected-text tools, Markdown notes, dictionary, and on-device dictation.
- Raycast JSON export migration, script compatibility, and saved window layouts on macOS.

Core features run locally without a Kebloop account. Currency conversions fetch and cache exchange rates; AI handoff sends your query to the provider you choose.

## Build and install

- [macOS build and installation](docs/build-macos.md) — macOS 14+, with Xcode 27+ to build.
- [Linux build and installation](docs/build-linux.md) — requirements, installation, packaging, and development commands.

## Getting started

On macOS, press **Option-Space** to open the launcher. On Linux, bind your preferred shortcut to `kebloop toggle`; see the [Linux shortcut guide](linux/README.md#bind-the-launcher-shortcut).

Type to search, use **Up/Down** to select a result, **Return** to open it or copy a calculation, and **Escape** to close. Search for **Clipboard History**, **Quicklinks**, **Emoji & Symbols**, or **Open Settings** to open a command.

## Documentation

- [Usage guide](docs/usage.md): commands, calculator syntax, privacy, and macOS settings.
- [Linux guide](linux/README.md): desktop integrations, shortcuts, storage, and platform differences.
- [Conversions](docs/conversions.md) and [automation](docs/automation.md).
- [Performance profiling](docs/performance.md), [feature roadmap](docs/roadmap.md), and [verification report](docs/feature-verification.md).

## License

Kebloop is released under the [MIT License](LICENSE). The bundled Unicode emoji data is covered by the [Unicode License](Sources/KebloopCore/Resources/unicode-license.txt).
