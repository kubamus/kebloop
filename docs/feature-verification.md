# Milestones 3–4 feature verification

Verified on 2026-10-12 on arm64 macOS 27, using release builds. All tests use isolated preferences, synthetic named pasteboards and temporary files. Personal scripts, shortcuts, clipboard history, and privacy permissions were not modified for testing.

The combined release suite reports **153 tests, zero failures, two opt-in timing tests skipped**. The separate timing profile passes (query median 4.825 ms, p95 12.716 ms; navigation median 0.081 ms). Clipboard tests pass 23 cases; automation tests pass 23 cases; the core suite passes 30 cases with its timing case skipped.

The feature checks below exercise the production stores, parser, subprocess runner, snippet presenter, command registry, and AppKit launcher/action-menu code. Automated AppKit tests do not establish actual VoiceOver speech, physical keyboard layout behavior, or permission-prompt usability.

| Feature | Checks performed | Evidence |
| --- | --- | --- |
| Clipboard count/age retention | Count and age expiration, persisted storage cleanup, current settings applied during initial load | ClipboardHistoryTests |
| Clipboard pins | Pins exempt from count/age, pin limit, freed pin slot, clear includes pins | ClipboardHistoryTests |
| Recording pause | Pause survives relaunch, expired deadline resumes, paused clipboard is not backfilled | ClipboardHistoryTests |
| Privacy/source exclusions | Excluded attributed app, unknown source, sensitive markers suppress text/image/rich formats | ClipboardHistoryTests |
| Clipboard actions | Copy/pin/remove applicability, keyboard navigation through actual action controller, remove pinned/unpinned selected item, preserve query and remaining results | ResultActionTests, MilestoneIntegrationTests |
| Clipboard image/rich formats | Image thumbnail decoding, image/RTF/HTML restoration, text fallback, per-entry/aggregate limits, migration, missing/corrupt attachment recovery, remove/clear/expire file cleanup | ClipboardHistoryTests |
| Snippets | Named create/edit/delete, validation/size bounds, repeated/Unicode/nonrecursive placeholders, missing values, production dialogs with injected responses, preview text, cancel at each step, copy without permission, trusted insertion decision and untrusted copy fallback | SnippetTests |
| Portable snippets/profiles | Legacy configuration decode, export, merge/replace and backup, active profile switching | SnippetTests |
| Unit conversions | Every documented alias, length/mass/temperature/data dimensions, offsets/absolute zero, connectors, decimal commas, locale display, incompatible units, incomplete input | CalculatorTests |
| Date math | Difference/addition syntax, relative dates, locale-ambiguous dates, offsets/Z, current time zone, DST, incomplete input | CalculatorTests |
| Localization | Matching English/Polish catalog keys, valid resources, coverage for all literal/dynamic lookup keys, direct packaged Polish lookup, English and Polish command aliases | LocalizationTests plus catalog audit |
| Command shortcuts | Snippet/script/extension IDs persist, registration released on disable/remove, re-enable registration, Unicode event label with stable hardware key code, recorder cancellation | UserCommandShortcutTests |
| Launcher integration | Enabled/disabled snippet/script/extension results, dynamic command invocation routing, category toggle, Polish aliases | MilestoneIntegrationTests |
| Scripts | Literal base and invocation arguments, success/output, missing runtime/nonzero exit, separate streams/combined bound, timeout/cancel, ordinary descendant cleanup, disable/delete/edit/reload while running, no stale completion, minimal environment | AutomationTests |
| Extension API | Real sample search/result/detail/action/preference round trip, supported version, malformed/incompatible/duplicate/oversized responses, input limits, slow execution keeps main actor responsive, superseded responses discarded, disable/delete/update/reload cancellation and stale state cleanup | AutomationTests |
| Automation import/privacy | Import never executes code, all profiles disabled for review, merge/replace backup, malformed imports atomic, local preference values excluded from exports | AutomationConfigurationTests |
| Responsiveness | Release AppKit typing/navigation profile and standalone file visibility benchmark | performance/feature-verification-results.json |
| Packaging | Release app build, localization bundle included, signature/Info.plist validation | scripts/build-app.sh |

## Fixes found during verification

- Added Remove Item to Clipboard History’s ⌘K menu, including a trash icon and Polish translation. Deleting a pinned or unpinned selection retains the search query.
- Extension updates and reload/profile changes retained stale result/detail/message state. Installation, reload, disable, and removal now clear that state; delayed responses cannot restore it.
- Rich clipboard payloads were dropped when AppKit synthesized plain text above the text limit. Capture now omits oversized plain text while retaining bounded images/RTF/HTML; plain-only oversized entries remain rejected.
- Closing or aborting a snippet preview could be interpreted as Copy. The presenter now copies only for an explicit Copy or Insert response; cancellation/abort leaves the pasteboard unchanged.

## Native UI coverage still pending

Restarting the rebuilt app restored native launcher inspection and confirmed the new commands are present. Opening Settings then closed the native automation pipe again; selecting Kebloop by bundle identifier or app path subsequently timed out. A process sample shows the app’s main thread idle in the normal AppKit event loop, rather than blocked by script/extension work. Resetting the automation session did not restore access to the Settings window. This prevents live verification of settings forms, mouse activation, actual insertion into another app, Accessibility permission prompts, VoiceOver speech, IME composition, physical Polish/non-US shortcut recording, translated layout at large text sizes, and display/Spaces behavior.

The earlier smoke check verified root command visibility, `1 km to m` → `1000 m`, and `ustawienia` → Open Settings. Those checks do not cover the remaining live interactions. Source attribution remains an intentional foreground-app approximation; deliberately detached subprocess descendants remain outside the runner’s lifecycle group.


## Everyday workflow priorities (2026-10-13)

The six suggested priorities are implemented. `swift test -c release` executes 177 tests across Kebloop and KebloopCore: 175 pass, two opt-in timing tests are skipped, and there are zero failures. The release bundle builds and passes the build script's signing verification. `git diff --check` passes.

| Feature | Automated verification |
| --- | --- |
| Clipboard/emoji direct paste and plain text | Permission and secure-input gates; intended foreground app; clipboard revision; cancellation and invalid-target replacement; action applicability; retained clipboard removal workflow |
| Running app controls and focus/hide hotkeys | Service builds; running-dependent action availability; shared hotkey registration/routing regression coverage; restart implementation waits for normal termination |
| System actions | Restart/logout event IDs use macOS confirmation, not immediate logout |
| Basic window management | Visible-frame halves, center, coordinate conversion for displays above/left, next-display clamping, greatest window/display intersection |
| Calculator | Unit and duration conversions, mixed-unit addition/subtraction, known unit/currency ambiguity, named time zones, dated currency result with clean clipboard output, cache limits/expiry, mocked endpoint/response validation, request coalescing, stale offline fallback, persisted accepted history and distinct launcher identities |
| Snippets | Legacy decoding, dynamic values/defaults, literal substitution, keyword uniqueness and boundaries, unresolved-trigger preservation, bounded buffer, deletion count excluding the suppressed delimiter, retained preview/cancel behavior |

Native automation obtained an initial snapshot of the previously running app, then repeatedly returned `timeoutReached` for both AX and screenshot requests; reconnecting with the release bundle after a session reset also timed out. This is not native acceptance of the new release. Direct paste into real apps, typing expansion, app hide/restart behavior, permission dialogs, VoiceOver, physical global hotkeys, multi-display window moves, and supported-macOS coverage remain pending native verification. No force quit, restart/logout/sleep, or actual keystroke insertion was executed against user applications by the tests. Typing expansion remains off by default.

The larger candidates are saved in the [roadmap](roadmap.md#larger-feature-backlog).

## Larger workflow delivery — 2026-10-13

The macOS release suite executes **228 tests**: **226 pass**, **two opt-in timing tests are skipped**, and there are **zero failures**. The separately enabled AppKit responsiveness profile passes; [measurements](performance.md#larger-workflow-follow-up) include the empty-panel adjustment. The release app bundle builds and passes signature and plist verification.

| Workflow | Delivered behavior and automated coverage |
| --- | --- |
| Window/menu search | Bounded background AX scans of running app windows and the previous app's enabled menus; stale/permission errors, selection and Return activation. Isolated hosting tests cover unavailable app/access feedback. |
| Calendar/meetings | Explicit EventKit opt-in, selected calendars, next meeting, explicit join; tests cover declined/canceled/all-day/finished filtering, provider-link priority, and rejecting unsafe schemes. |
| Selected-text actions | AX reading preserves the clipboard; explicit local/cloud provider selection, instruction/data separation, preview/copy/confirmed replacement, exact field/range and clipboard gates, stale-response rejection. URLProtocol tests validate requests/responses/errors; credentials use a disposable Keychain namespace in tests. |
| Markdown notes | Floating local editor, search, atomic Markdown autosave, recovery choices and export. Tests cover immediate-relaunch recovery, restoration, discard, limits, and unsafe draft IDs. |
| Dictionary/dictation | Native dictionary definitions and Dictionary app fallback; explicit on-device speech recording, bounded sessions/finalization, preview/copy/insert. Tests cover unsupported recognition without permission requests and copy fallback without a target. |
| Raycast migration | Selective, additive JSON snippet/quicklink import with validation/conflicts. Tests verify no partial application of invalid batches, preserved settings, duplicate skipping, and omitted typing triggers. |
| Raycast compatibility | Reviewed disabled imports of real Script Command metadata and standalone no-view JavaScript with unchanged SDK imports. Fixtures run original scripts, preference/clipboard feedback, and UUID copying through Node crypto; unsupported SDK imports discard native effects. See [the exact subset](raycast.md). |
| Saved window layouts | Persistent display-relative frames, bundle/title/duplicate matching, bounded background restore, missing-app/window and partial-failure reports. Tests cover geometry, matching, persistence, and isolated view creation. |

Native UI automation timed out twice while selecting the packaged app. Live permission prompts, selected-text replacement, audio recording, dictionary content on user dictionaries, calendar joining, keyboard/VoiceOver behavior, multiple displays/Spaces, and supported-macOS coverage remain native acceptance work. Unit tests do not establish those behaviors on physical configurations. No personal calendars, selections, audio, clipboard data, or provider credentials were used for execution tests.

[The roadmap](roadmap.md) now contains unfinished work only, including native acceptance, signing/updates, broader Raycast compatibility, remaining translations, and later calendar/workspace additions. Linux workflow equivalents remain separate work.

## Keyboard-first utilities follow-up — 2026-10-13

The final release suite executes **236 tests**: **234 pass**, two opt-in timing tests are skipped, and there are zero failures. Added coverage checks inline command routing, `define word` and `notes query` parsing, note selection without creating another note, stale dictionary response rejection, Escape returning to launcher search, and Markdown block/inline rendering. The separately enabled responsiveness profile passes; see [measurements](performance.md#keyboard-first-utilities-follow-up).

Notes now browse within the launcher, open a floating rendered preview, and show source only while editing. Keyboard operations cover create, find, navigate, edit, save/preview, cancel/close, confirmed deletion, export, and recovery choices. Drafts rejected by storage limits cannot be silently replaced by opening another note or quitting. Dictionary and Dictation are inline launcher modes and also appear in the menu-bar menu. Recording still starts explicitly, respects composition, and cancels when the launcher is dismissed. Native focus, permission prompt, microphone, VoiceOver, and physical keyboard acceptance remains pending on this host; unit coverage is not being substituted for those checks.

## Simpler floating notes — 2026-10-13

Floating notes now use one pane with a small action header and visible shortcut hints. New notes are blank and focus the native text editor immediately; existing notes open as Markdown previews. Return edits, Escape saves and returns to preview, ⌘W saves and closes, and ⌘P saves and opens the launcher’s notes browser. Native find uses ⌘F and ⌘G/⇧⌘G, with Escape dismissing the find bar first. Editing preserves native text selection, line deletion, and undo/redo. Failed saves retain the draft and block switching or closing.

The release suite executes **242 tests**: **240 pass**, two opt-in timing tests are skipped, and there are zero failures. Added AppKit hosting tests verify editor focus, end-of-note insertion, bound draft updates through typing and undo/redo, and find requests issued before window attachment. Session tests cover Escape saving, blank-note creation, find from preview, and rejected drafts preventing close or replacement. These isolated AppKit checks do not establish physical keyboard, VoiceOver, or permission-prompt acceptance; live native automation remains unavailable on this host.

## Linux workflow parity — 2026-10-13

The Rust port now exposes the workflow commands through launcher search, CLI/compositor bindings, and relevant tray entries. Notes use a reusable floating preview/editor with keyboard creation, find, browse, undo/redo, recovery, export, and guarded close. Dictionary and dictation are inline launcher pages. Calendar selection uses local iCalendar files; menu/selection actions use AT-SPI, and window search/layouts use existing compositor backends. Raycast migration and the reviewed Script Command/no-view SDK subset are implemented in Rust. See [Linux usage and exact support](../linux/README.md#workflow-commands).

Validation on the existing Ubuntu ARM64 Linux VM:

- `cargo test --workspace --locked`: **570 pass**, **20 opt-in desktop tests ignored**, zero failures.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`, workspace formatting, and `cargo build --release --locked` pass.
- The separately enabled private D-Bus selected-text test passes, including Unicode insertion, changed range/text/focus/password/PID refusal, and rollback. It exposed stale AT-SPI property caching; workflow proxies now read fresh properties.
- `tests/gtk_workflows.py` passes against the release binary in private Xvfb/D-Bus/Openbox with AT-SPI. Real keystrokes verify new-note focus, multiline typing, persisted source, rendered fenced code/table preview, undo, find/Escape, browser/new/switch, close, save-before-quit/failure retention, and dictionary/dictation remaining inline without starting recording.
- Service fixtures verify atomic notes/recovery, bounded calendar files and recurrence across DST, additive migration conflicts, SDK batch validation/execution, partial/ambiguous layout matching, selected-text provider requests, and stopping/cancelling dictation with private temporary audio cleanup.

The GTK smoke uses synthetic notes and does not record real audio, connect calendar accounts, call paid AI providers, or use personal selections. Real microphone devices/models, account synchronization, screen readers/input methods, physical monitor/Spaces equivalents, and apps lacking accessible AT-SPI objects remain acceptance or platform follow-up. Local calendar support includes daily/weekly recurrence; other rules and recurrence exceptions require expanded exports. Saved geometry is bounded by compositor support and does not restore workspace/tiling topology. These gaps remain on [the roadmap](roadmap.md).
