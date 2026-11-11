# Build Kebloop for macOS

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

The bundle includes the separately signed **Kebloop New File** Finder Sync extension. Enable it once in macOS extension settings to add Finder context-menu and toolbar actions; see [New files in Finder](usage.md#new-files-in-finder-macos). A plain `swift build` builds the launcher only; the packaging script compiles and embeds the extension.

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

## Development

Open `Package.swift` in Xcode, or run these commands from the repository root:

```sh
swift build
swift test -c release
./scripts/test-finder-settings.sh
```

The Finder check runs a signed process with the extension's sandbox entitlements. It verifies that Finder reads the same configuration as the launcher, keeps customization inside the submenu, and creates real files when receiving menu actions with only numeric tags. It reads your saved options without changing them and creates disposable test files in a temporary folder. It uses `KEBLOOP_SIGNING_IDENTITY` when supplied, just like the build script.

See the [usage guide](usage.md) for shortcuts, permissions, and settings.
