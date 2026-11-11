#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd -P)"
DIST_DIR="$REPO_ROOT/dist"
APP_PATH="$DIST_DIR/Kebloop.app"
BUNDLE_ID="com.kebloop.launcher"

fail() {
  printf 'build-app: %s\n' "$*" >&2
  exit 1
}

[[ "$(uname -s)" == "Darwin" ]] || fail "this build script requires macOS."

SDK_VERSION="$(xcrun --sdk macosx --show-sdk-version 2>/dev/null)" \
  || fail "could not find the macOS SDK. Install Xcode 27 or newer and select it with xcode-select."
SDK_MAJOR="${SDK_VERSION%%.*}"
[[ "$SDK_MAJOR" =~ ^[0-9]+$ ]] || fail "could not parse macOS SDK version '$SDK_VERSION'."
(( SDK_MAJOR >= 27 )) || fail "Kebloop uses macOS 27 SDK APIs. Install Xcode 27 or newer (found SDK $SDK_VERSION)."

cd -- "$REPO_ROOT"
[[ ! -L "$DIST_DIR" ]] || fail "refusing to write through symlinked output directory: $DIST_DIR"
mkdir -p -- "$DIST_DIR"

TEMP_DIR="$(mktemp -d "$DIST_DIR/.Kebloop-build.XXXXXX")"
STAGED_APP="$TEMP_DIR/Kebloop.app"
PREVIOUS_APP="$TEMP_DIR/previous.app"
HAD_PREVIOUS=0
COMMITTED=0
KEEP_TEMP=0

cleanup() {
  local status=$?
  trap - EXIT
  if (( HAD_PREVIOUS && ! COMMITTED )) && [[ ! -e "$APP_PATH" && -d "$PREVIOUS_APP" ]]; then
    if ! mv -- "$PREVIOUS_APP" "$APP_PATH"; then
      printf 'build-app: could not restore the previous app bundle; it remains at %s\n' "$PREVIOUS_APP" >&2
      KEEP_TEMP=1
      status=1
    fi
  fi
  if (( ! KEEP_TEMP )); then rm -rf -- "$TEMP_DIR"; fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

swift build -c release
BIN_PATH="$(swift build -c release --show-bin-path)"
BUILT_EXECUTABLE="$BIN_PATH/Kebloop"
[[ -x "$BUILT_EXECUTABLE" ]] || fail "release executable not found at $BUILT_EXECUTABLE"

mkdir -p -- "$STAGED_APP/Contents/MacOS" "$STAGED_APP/Contents/Resources"
cp -- "$BUILT_EXECUTABLE" "$STAGED_APP/Contents/MacOS/Kebloop"
for resource_bundle in "$BIN_PATH"/Kebloop_Kebloop.bundle "$BIN_PATH"/Kebloop_KebloopCore.bundle; do
  [[ ! -d "$resource_bundle" ]] || cp -R -- "$resource_bundle" "$STAGED_APP/Contents/Resources/"
done
cp -- "$REPO_ROOT/assets/Info.plist" "$STAGED_APP/Contents/Info.plist"

SIGNING_IDENTITY="${KEBLOOP_SIGNING_IDENTITY:-}"
if [[ -z "$SIGNING_IDENTITY" ]]; then
  SIGNING_IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | awk '/"Apple Development:/ && !found { identity = $2; found = 1 } END { if (found) print identity }')"
fi
if [[ -z "$SIGNING_IDENTITY" ]]; then
  SIGNING_IDENTITY="-"
  printf 'No Apple Development identity found; using an ad-hoc signature. Accessibility permission may need to be granted again after rebuilding.\n'
fi

EXTENSION_PATH="$STAGED_APP/Contents/PlugIns/KebloopFinderExtension.appex"
mkdir -p -- "$EXTENSION_PATH/Contents/MacOS"
cp -- "$REPO_ROOT/FinderExtension/Info.plist" "$EXTENSION_PATH/Contents/Info.plist"
xcrun swiftc -O -swift-version 5 -parse-as-library -application-extension \
  -module-name KebloopFinderExtension -target "$(uname -m)-apple-macosx14.0" \
  -framework AppKit -framework FinderSync -Xlinker -e -Xlinker _NSExtensionMain \
  "$REPO_ROOT/Sources/KebloopCore/NewFile.swift" \
  "$REPO_ROOT/FinderExtension/NewFileMenu.swift" "$REPO_ROOT/FinderExtension/FinderSync.swift" \
  -o "$EXTENSION_PATH/Contents/MacOS/KebloopFinderExtension"
codesign --force --sign "$SIGNING_IDENTITY" \
  --entitlements "$REPO_ROOT/FinderExtension/KebloopFinderExtension.entitlements" "$EXTENSION_PATH"
/usr/bin/plutil -lint "$EXTENSION_PATH/Contents/Info.plist" >/dev/null
codesign --force --sign "$SIGNING_IDENTITY" "$STAGED_APP"
codesign --verify --deep --strict "$STAGED_APP"
/usr/bin/plutil -lint "$STAGED_APP/Contents/Info.plist" >/dev/null
ACTUAL_BUNDLE_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$STAGED_APP/Contents/Info.plist")"
[[ "$ACTUAL_BUNDLE_ID" == "$BUNDLE_ID" ]] || fail "unexpected bundle identifier '$ACTUAL_BUNDLE_ID' in assets/Info.plist"
[[ -x "$STAGED_APP/Contents/MacOS/Kebloop" ]] || fail "staged app executable is missing or not executable"

if [[ -L "$APP_PATH" ]]; then
  fail "refusing to replace symlinked app bundle: $APP_PATH"
elif [[ -e "$APP_PATH" ]]; then
  [[ -d "$APP_PATH" ]] || fail "refusing to replace non-directory output: $APP_PATH"
  [[ -f "$APP_PATH/Contents/Info.plist" ]] || fail "refusing to replace an app without Contents/Info.plist: $APP_PATH"
  EXISTING_BUNDLE_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP_PATH/Contents/Info.plist" 2>/dev/null || true)"
  [[ "$EXISTING_BUNDLE_ID" == "$BUNDLE_ID" ]] || fail "refusing to replace an app with bundle identifier '${EXISTING_BUNDLE_ID:-unknown}': $APP_PATH"
  mv -- "$APP_PATH" "$PREVIOUS_APP"
  HAD_PREVIOUS=1
fi

if ! mv -- "$STAGED_APP" "$APP_PATH"; then
  fail "could not move the verified app bundle into place: $APP_PATH"
fi
COMMITTED=1
printf 'Built and verified %s\n' "$APP_PATH"
