#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd -P)"
TEST_DIR="$(mktemp -d /private/tmp/kebloop-finder-settings.XXXXXX)"
trap 'rm -rf -- "$TEST_DIR"' EXIT

# Use the same identity and container as the packaged Finder extension. This
# reads existing preferences without changing them or registering another plugin.
SIGNING_IDENTITY="${KEBLOOP_SIGNING_IDENTITY:-}"
if [[ -z "$SIGNING_IDENTITY" ]]; then
  SIGNING_IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | awk '/"Apple Development:/ && !found { identity = $2; found = 1 } END { if (found) print identity }')"
fi
SIGNING_IDENTITY="${SIGNING_IDENTITY:--}"

PROBE_APP="$TEST_DIR/Probe.app"
mkdir -p -- "$PROBE_APP/Contents/MacOS"
cat > "$PROBE_APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.kebloop.launcher.finder</string>
<key>CFBundleExecutable</key><string>Probe</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST
xcrun swiftc -swift-version 5 -parse-as-library \
  "$REPO_ROOT/Sources/KebloopCore/NewFile.swift" "$REPO_ROOT/FinderExtension/NewFileMenu.swift" \
  "$SCRIPT_DIR/testing/FinderSettingsProbe.swift" "$SCRIPT_DIR/testing/FinderMenuChecks.swift" \
  -o "$PROBE_APP/Contents/MacOS/Probe"
cp -- "$PROBE_APP/Contents/MacOS/Probe" "$TEST_DIR/HostProbe"
codesign --force --sign "$SIGNING_IDENTITY" \
  --entitlements "$REPO_ROOT/FinderExtension/KebloopFinderExtension.entitlements" "$PROBE_APP"
"$TEST_DIR/HostProbe" > "$TEST_DIR/host.json"
"$PROBE_APP/Contents/MacOS/Probe" --expect-sandbox > "$TEST_DIR/finder.json"
if ! cmp -s "$TEST_DIR/host.json" "$TEST_DIR/finder.json"; then
  printf 'Finder and host settings differ.\n' >&2
  exit 1
fi
printf 'Finder settings match the host in a signed sandbox.\n'
"$PROBE_APP/Contents/MacOS/Probe" --test-menu "$TEST_DIR/files"
