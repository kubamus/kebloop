# Kebloop remaining roadmap

Updated: 2026-10-13. This document contains unfinished work only. Implemented workflows are documented in [usage](usage.md), [automation](automation.md), and [Raycast compatibility](raycast.md).

## Native acceptance verification

- Complete VoiceOver announcements, keyboard traversal, focus restoration, and action-menu Escape checks across launcher, settings, and new workflow windows.
- Verify large text, Polish translations, non-US shortcuts, input-method composition, reduced transparency, and increased contrast on supported macOS versions.
- Test global hotkeys, permission denial/revocation, real clipboard/snippet insertion, selected-text replacement, on-device speech recording, and calendar access in native apps.
- Test multiple displays, full-screen Spaces, disappearing windows, and saved layout restoration after display removal on physical configurations.
- Verify configuration file panels, profile switching, actual Apple Shortcuts, and script/extension views with native keyboard and accessibility navigation.

Automated tests and local smoke checks do not replace these device checks. Prior coverage is recorded in [feature verification](feature-verification.md) and [performance](performance.md).

## Signed and notarized distribution

- Produce a repeatable distribution build, sign with a distribution identity, notarize, staple, and publish versioned downloads and installation instructions.
- Verify Intel/Apple Silicon packaging, supported macOS versions, permission persistence, and preference/history retention when upgrading.
- Test installation on a clean Mac without developer tools and publish the tested support matrix and release notes.

Requires Apple Developer credentials and a chosen download destination. Publication requires an explicit release instruction.

## Built-in updates

- Select and integrate a maintained macOS updater with manual checks, optional automatic checks, signed metadata/artifacts, and release notes.
- Choose and configure the update-feed host.
- Test an actual upgrade between signed builds, including invalid signatures, broken downloads, offline use, interruptions, and configuration retention.

Depends on signed distribution and release hosting.

## Calendar additions

- Create calendar events from the launcher.
- Evaluate automatic joining and camera previews with explicit user controls.

## Raycast compatibility beyond the delivered subset

- Support encrypted `.rayconfig` backups only after establishing a documented, verifiable format and decryption workflow. Individual JSON snippet/quicklink exports are already supported.
- Extend the delivered no-view JavaScript `@raycast/api` subset with React rendering, broader SDK APIs, external dependencies, and credential support. Validate against useful real extensions and define supported API versions, lifecycle, distribution, and memory costs before claiming full SDK compatibility.
- Extend script compatibility to argument forms, inline refresh, and other currently rejected metadata with equivalent behavior.
- Decide separately whether to offer a marketplace or unattended installation.

See [the supported subset and explicit exclusions](raycast.md).

## Notes and workspaces

- Add Markdown formatting tools and integrations after validating local file storage and recovery in everyday use.
- Add automatic workspace switching after validating saved layout matching and partial restore reports.

## Localization follow-up

- Complete Polish translations of new workflow screens and dynamic diagnostics; audit long translated strings and accessibility labels.
- Verify right-to-left layout before adding a right-to-left language.

## Platform follow-up

- Add Linux calendar account synchronization and recurrence rules/exceptions beyond the local daily/weekly `.ics` subset.
- Add dictation providers beyond local whisper.cpp/ALSA, and verify real microphone devices and permissions across desktops.
- Verify the new GTK workflow windows with screen readers, physical keyboards/input methods, multiple monitors, and supported GNOME/KDE/wlroots versions.
- Extend menu/selected-text support for apps that do not expose AT-SPI actions or editable text.
- Restore Linux workspace/tiling topology and arbitrary window geometry on compositors that currently do not expose those operations.
