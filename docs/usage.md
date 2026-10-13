# Kebloop usage guide

For build and installation instructions, see the [macOS guide](build-macos.md) or [Linux guide](build-linux.md).

## Using Kebloop

- **Option-Space** opens or closes the launcher by default (your saved shortcut is retained).
- Type an app name, alias, quicklink, or filename. Apps and commands appear immediately; indexed files appear underneath. **Up/Down** selects a result; **Return** opens it or brings it forward. The launcher hides as soon as an external action is accepted, without waiting for the destination app to finish launching. Failed launches restore the query with an error unless you have already opened a new launcher session.
- Type math in the same field. Results appear immediately; **Return** copies the answer and closes the launcher.
- **Command-K**, the **Actions** button, or right-clicking a result opens its custom action panel. **Up/Down** selects an action and **Return** runs it; **Escape** closes the panel while keeping your search and selected result.
- **Escape** or an outside click closes the launcher.
- **Command-comma** opens settings. The menu-bar icon also opens the launcher, settings, and a manual app refresh.
- Settings offer Option-Space, Control-Space, Command-Shift-Space, Command-Space, or a recorded launcher shortcut, plus optional launch at login. System shortcuts may need to be reassigned before using the same combination.

## Commands and settings

Type a command name in the launcher, then press Return:

| Command | Behavior |
| --- | --- |
| Clipboard History | Search recent text, images, and rich text; Return pastes into the previous app, Command-Return copies, and Command-Shift-Return pastes plain text. Trash removes an item; Clear… removes all after confirmation. |
| Quicklinks | Browse saved links. Links also appear directly in root search by name. |
| Web Search | Search the web using a configured template; the destination opens only after you choose the result. |
| Apple Shortcuts | Browse the shortcuts selected in Settings → Apple Shortcuts. |
| Profiles | View and switch saved settings profiles. |
| Emoji & Symbols | Open the searchable emoji grid with categories and recents; click or Return pastes an emoji; Command-Return or the Copy button copies it. Arrow keys navigate. |
| Browse Commands / Help | Search built-in commands, saved quicklinks, selected Apple Shortcuts, current shortcut assignments, and feature availability. |
| Welcome to Kebloop | Replay the skippable first-run choices for login startup, clipboard history, and optional Siri access. |
| Open Settings | Configure Kebloop without using its menu-bar icon. |
| Shut Down… | Type `shutdown`, `power off`, or `turn off`, then Return to open the macOS shutdown confirmation. Confirm there to turn off your Mac, or cancel. |

Within clipboard and quicklink search, Escape or the back button returns to root search. Lists scroll; arrow navigation keeps the selected row visible.

**Quicklinks:** Settings → Quicklinks creates, edits, and removes named `https://` or `http://` URLs and absolute or `~/` file/folder paths. An optional one-word keyword accepts a query after it; templates substitute percent-encoded text for `{query}`. Unsupported schemes, invalid templates, duplicate keywords, and empty names are rejected.

**Aliases:** Settings → Aliases assigns an alternate search name to an app. Exact aliases outrank exact app names: Ghostty with alias `terminal` appears above Terminal. Aliases persist and duplicate aliases are rejected.

**App shortcuts:** An app shortcut focuses or launches its app; pressing it while the app is frontmost hides the app. Settings → App Shortcuts lets you search an installed app and record a physical key combination containing Command, Option, or Control. Assignments persist across restarts. Duplicate assignments, launcher conflicts, and unavailable system shortcuts produce feedback. Recording temporarily releases global shortcuts and restores them when finished or cancelled.

**Command shortcuts:** Settings → Command Shortcuts assigns global shortcuts to built-in commands, saved quicklinks, and selected Apple Shortcuts. These use the same conflict checks as app shortcuts. Removing a quicklink or deselecting an Apple Shortcut removes its command binding. Settings → Help shows the current assignments; its shortcut-hints choice controls the hints shown beside launcher actions.

**Search and appearance:** Settings → Search enables or disables app, file, link, and built-in-command results; chooses searchable folders; excludes folders by path; hides selected apps; manages pinned favorites; and selects the empty-launcher list. Folder exclusions respect path boundaries, so excluding `~/Documents/Private` does not hide `~/Documents/Privateer`. Reset Search Settings restores search defaults and removes pinned favorites. Settings → Appearance selects system, light, or dark appearance; text size; panel width; row density; transparency; and pointer, active-window, or selected-display placement. Reduced transparency follows the macOS accessibility preference, and an unavailable selected display falls back to the pointer display.

**Parameterized quicklinks and web search:** Settings → Quicklinks accepts an optional one-word keyword. Type the keyword followed by text to pass an argument, such as `gh kebloop`; `{query}` in a URL template is replaced with percent-encoded text. Web search templates require an HTTP or HTTPS URL containing `{query}` and show a preview in settings. Unmatched text offers a web-search result only when a valid template is configured; opening the destination requires selecting that result.

**Apple Shortcuts:** Settings → Apple Shortcuts refreshes the shortcuts available on this Mac and lets you choose which appear in Kebloop. You can run a selected shortcut with optional text input; a shortcut can be cancelled from settings. Kebloop uses macOS's `shortcuts` command-line tool. macOS may request permission on first use, and shortcuts missing on another Mac are reported as unavailable.

**Profiles and portable configuration:** Settings → Profiles creates, duplicates, renames, switches, exports, and deletes profiles. Each profile contains links, aliases, command bindings, pins, and search and appearance preferences; at least one profile is always kept. Login startup and clipboard privacy remain global. Settings → Configuration exports configuration as JSON, or imports it with a preview and explicit merge or replace choice. Replace saves a backup first. Clipboard contents, credentials, caches, and usage history are excluded. The preview reports malformed or unsupported files, conflicts, unavailable paths or apps, and Apple Shortcut names that may be missing on another Mac.

**First run and help:** The short welcome screen lets you choose login startup and clipboard recording, and offers Accessibility access for direct Siri text submission as an optional choice. Skip or finish the walkthrough to dismiss it; type `welcome` or open Settings → Help to replay it or browse commands. Help searches current shortcut assignments and selected links, and reflects disabled clipboard recording or an empty Apple Shortcuts selection. You can hide shortcut hints there without removing your assignments.

**Clipboard:** Text copied after Kebloop starts is recorded locally; the existing clipboard is not ingested on startup. Duplicate text moves to the front, and entries larger than 100 KB are skipped. Concealed, transient, autogenerated, and password-marked clipboard content is excluded. Settings → Clipboard pauses recording or clears saved history. Use a row’s trash button or Command-Backspace to remove an item. Clear… or Command-Shift-Backspace opens a confirmation to clear all saved history. Image and rich-text history, pins, and retention settings are described below.

**Files:** Filename search runs directly from root search, with apps above files and no separate command to open. A 35 ms debounce starts Spotlight searches; cached query snapshots appear immediately while narrowing or revisiting a query, and new results stream during gathering. Hidden paths, app package contents, `~/Library`, and `node_modules` are excluded. Temporary directories, editor backup/swap files, and partial downloads are also excluded. Git repositories are checked against their `.gitignore` rules before either cached or fresh results appear, including nested rules, negations, worktrees, and configured excludes. This uses local `/usr/bin/git` (provided by Xcode or Command Line Tools); tracked files follow Git’s own ignore semantics. Checks run on a worker queue and rule changes apply on the next search. Exact names and prefixes rank before substring matches, with a boost for Documents/Desktop/Downloads/Public. Results depend on Spotlight indexing and macOS access restrictions. This is filename search, not full-content search; the service keeps up to 40 results per query and root search displays up to 12.

## AI handoff

Settings → General → AI provider selects **OpenAI (ChatGPT)**, **Claude**, or **Local (Siri)**. Type a query in root search and press **Tab**, or click the compact Ask button. Your selection persists across restarts.

ChatGPT opens with your query filled in using a prompt-bearing URL. Claude also receives the query in its URL; signing in may be required. Siri opens a new conversation, fills the native query field, and submits it directly. Enable Siri and Type to Siri, then allow Kebloop under System Settings → Privacy & Security → Device Control and Data Access (Accessibility on older macOS) once. Development builds use an available Apple Development signing identity to preserve grants across rebuilds; set `KEBLOOP_SIGNING_IDENTITY` to choose one. Without a signing identity, the ad-hoc fallback requires renewing the grant after each rebuild. No API keys are required.

Finder is indexed explicitly from CoreServices, alongside the user-facing CoreServices applications, so it appears in app search.

AI handoff text is marked transient so Kebloop does not add it to clipboard history. Blank queries and modified Tab shortcuts keep their normal behavior. If a destination fails to open, the launcher restores the query and shows an error.

## Calculator

Examples: `2*(3+4)`, `2^10`, `sqrt(81)`, `abs(-2)`, `round(2.6)`, `15% of 200`, `20% off 80`.

Supports `+ - * / ^`, parentheses, unary signs, scientific notation, `pi`, `e`, and decimal commas or dots. Powers associate right to left; `-2^2` is `-4`. `%` is a fraction: `200 + 10%` is `200.1`; use `10% of 200` for `20`. Functions take one argument. Unit/date calculations and cached currency conversions are described in the conversion guide.

Calculations use Double and display up to 16 significant digits with insignificant floating-point noise suppressed. Clipboard output uses the same ungrouped decimal-dot representation. Expression/token/depth limits bound evaluation; unfinished expressions stay quiet.

The panel uses native Liquid Glass on macOS 26+, with interactive glass on macOS 27. Older supported releases use a native popover material. The launcher has no branding or keybinding footer.

**Background lifecycle:** Command-Q hides Kebloop’s interface without quitting. Closing all windows leaves it running in the menu bar. The welcome screen asks whether to launch at login; macOS may require approval in System Settings → General → Login Items. An explicit menu-bar Quit command remains available, and system shutdown/logout can terminate the app normally.

## App discovery and storage

Scans `/Applications`, `/System/Applications`, `~/Applications`, and the system Cryptex application directory (Safari), including organizational subfolders. App bundles are not traversed for embedded helpers; hidden and background-only apps are excluded. Settings can hide chosen applications from launcher results. Cached results appear immediately; scans run in the background on startup, each launcher opening, every five minutes, and manual refresh. Native workspace icons are cached directly and prewarmed in small yielded batches; visible icons take priority.

Search matches exact names, prefixes, word prefixes, and fuzzy subsequences, with a small usage boost. File results are limited to the folders selected in Settings → Search, then filtered by user exclusions and built-in temporary and package rules. App metadata is cached in `~/Library/Caches/Kebloop/application-index.json`; shortcut preferences, profiles, app bindings, aliases, quicklinks, emoji recents, and launch counts use UserDefaults. Clipboard text history is stored in `~/Library/Application Support/Kebloop/clipboard-history.json` with owner-only permissions. No Accessibility permission is needed for local search, clipboard, links, or app shortcuts; direct Siri text submission requests it only if chosen.

## Verification

Release XCTest coverage checks matching/ranking, duplicate names, calculator grammar, invalid input, precision, emoji search, quicklink validation/persistence, clipboard limits/deduplication/privacy/persistence, file-query cancellation/caching/ranking, alias priority/persistence, clipboard removal, Carbon shortcut event routing, and AI provider persistence/clipboard handoff/target selection. A synthetic 1,000-app ranking benchmark averaged approximately 9 ms on the development Mac. This measures ranking, not full input-to-render latency or parity with Raycast.

Native UI smoke checks covered app/calculator workflows, Settings search, creating and opening a quicklink, recording/copying clipboard entries, finding an indexed file, emoji search/copy, and the app shortcut recorder. App-scoped UI automation does not deliver system-wide Carbon hotkey events, so physical keyboard invocation of global shortcuts needs a manual check; native registration and Carbon event-routing tests pass. Multiple displays, full-screen Spaces, VoiceOver, login startup, and the under-100-ms open target still need broader device testing.

## Everyday utilities (milestone 3)

### Clipboard retention, pins, and formats

Settings → Clipboard contains a 1–200 unpinned item limit, retention in days (0 keeps entries indefinitely), source app exclusions by bundle identifier, and 15-minute/one-hour recording pauses. A pause deadline survives relaunch. Privacy controls remain global across profiles. The source is approximated using the foreground app when polling; background writers and rapid app switching cannot be attributed reliably. Sensitive pasteboard markers are checked for text, images, and rich text.

In Clipboard History, select an entry and press ⌘K to copy, pin/unpin, or remove the selected item. Removing an item keeps the current search query and selects a remaining result. Up to 50 pins are exempt from count/age retention. Clear History removes **all entries, including pins and attachment files**. Old text-only history migrates without losing text. Image previews load on demand; copying restores saved image/RTF/HTML formats plus plain text when available. Attachments have a 5 MiB per-entry and 50 MiB aggregate budget; unsupported or oversized payloads are omitted, with plain text retained where possible. Missing/corrupt attachments are discarded and orphan files are cleaned up.

### Snippets

Settings → Snippets manages named text. Use explicit placeholders such as `Hello {name}, your order {order_id} is ready.` Names start with an ASCII letter and contain letters, digits, or underscores; repeated placeholders share a value. Search the snippet title or open the Snippets command. Return prompts for each value, then shows a selectable preview. Cancel leaves the pasteboard unchanged. Copy needs no permission. Insert copies first and pastes into the previous application when Accessibility is already allowed; otherwise it explains how to enable it and retains the copy fallback. Optional keyword expansion uses an explicitly enabled local typing monitor; it is off by default. Snippets are included in configuration/profile export, merge, replace, and switching.

### Conversions and dates

See [supported conversion/date syntax](conversions.md). Results appear alongside calculations and Return copies the answer. Units, dates, durations, and time zones work offline. Currency conversions use cached rates and refresh over the network when needed.

### Language and keyboard layouts

English and Polish UI resources ship with the app and follow macOS preferred languages (including the per-app language setting). Stable English command names remain searchable alongside Polish aliases, for example `settings` / `ustawienia`, `clipboard` / `schowek`, and `help` / `pomoc`. Conversion numbers and dates use locale formatting. Input composition bypasses launcher action keys while marked text is active. Shortcut recording uses the current event’s characters with stable hardware key codes, preserving assignments across layout changes. Native VoiceOver, Polish/non-US layout, IME, and longer-text checks are recorded separately from unit coverage; not all combinations have been verified.

For community translations, add `<language>.lproj/Localizable.strings` beside the English catalog in `Sources/Kebloop/Resources`, retain every English key, translate only values, and preserve placeholder syntax and shortcut symbols. Wrap new static UI strings with `L(...)`, add matching keys to both current catalogs, build with `scripts/build-app.sh` (which packages the resource bundle), and review the UI at large text sizes. User-defined names, stable IDs, paths, executable arguments, and extension protocol fields must never be translated. Runtime diagnostics and user/extension-provided text may retain their original language. Right-to-left languages require a native layout audit before release.

## User automation (milestone 4)

Settings → Script Commands lets you select a local executable/runtime, add literal arguments (one per line), set a timeout, and explicitly enable a reviewed command. Search its title to run it; a separate output window offers cancellation and copyable output. Settings → Extensions imports versioned manifests, lets you review executable/arguments, enable/disable/remove extensions, and edit nonsecret preferences. Enabled extensions appear in root search; opening one presents its native search/list/detail/action interface.

See [automation API, trust model, templates, and examples](automation.md). Imported configuration disables scripts and extensions in every profile until reviewed. Import never executes code. Metadata and literal arguments are portable, so never store secrets there. Credential fields are unsupported; extension preferences remain local and are excluded from exports. Local subprocesses run with your account’s privileges, including filesystem/network access. Process isolation keeps the UI responsive; it is not a sandbox. Core launcher workflows remain offline and usable when an extension fails.


## Direct paste, app controls, and system commands

In Clipboard History, Return pastes the saved formats into the application active before opening Kebloop. Command-Return or Actions → Copy retains the copy-only workflow. Command-Shift-Return or Actions → Paste as Plain Text removes formatting when a text representation exists. Emoji Return/click pastes; Command-Return or the Copy button copies. Direct insertion needs Accessibility permission and skips secure input. If permission, foreground focus, or the clipboard changes, insertion is cancelled with feedback; a successful copy remains available for manual paste. Snippet Insert uses the same focus checks.

Running apps show a Running label. Their action menu offers Hide, Quit, Force Quit, and Restart. Force Quit asks first; normal quit/restart lets the application handle unsaved documents, and restart stops if the application has not terminated within ten seconds. App hotkeys toggle focus/hide; launching from search continues to open the app normally.

Type Lock Screen, Sleep, Restart, Log Out, Toggle Mute, or Toggle Appearance. Restart and log out use macOS confirmation; shutdown keeps its existing confirmation. Lock requires Accessibility. Sleep and system appearance may require macOS Automation access to System Events. Command shortcuts can be assigned to each action. Actions report failures rather than claiming success.

## Window management

Use launcher commands for left/right/top/bottom halves, four corner quarters, left/center/right thirds, left/right two thirds, maximize, center, restore, and previous/next display. Launcher commands arrange the focused window of the app active before Kebloop opened; global window shortcuts act directly on the active app.

Open **Settings → Window Management** to record or remove each global keybinding. **Use Suggested Shortcuts** fills unassigned actions with Control–Option shortcuts: arrows for halves, U/I/J/K for quarters, D/F/G for thirds, E/T for two thirds, Return to maximize, C to center, Delete to restore, and Shift–Left/Right for displays. Existing bindings are preserved and conflicts with the launcher, apps, other commands, or macOS are skipped and reported. These bindings also appear in Command Shortcuts and are saved with your configuration.

Accessibility permission is required on use; the settings page shows its status and links to System Settings. Full-screen, minimized, modal-sheet, and fixed-size windows are skipped. Positions use the display's usable bounds, excluding the Dock/menu bar. Restore returns to the frame saved before the first arrangement; moving between displays clamps the window to the destination. Display switching requires at least two displays.

## Calculator history and snippet defaults

Copying a calculator result adds it to Calculator History. Typing alone never records a calculation. Open Calculator History to search expressions/results, copy a saved answer, or use Actions → Clear Calculator History. History is local and excluded from portable profiles/configuration.

Snippets accept `{name:friend}` as a default value and `{date}`, `{time}`, or `{clipboard}` as values resolved locally when invoked. Repeated prompted values are shared; dynamic values cannot have defaults. In Settings → Snippets, assign an optional keyword such as `;addr` and explicitly enable keyword expansion. Typing the keyword followed by a space replaces it only when all prompted values have defaults. Otherwise the typed text stays untouched. The monitor uses a bounded in-memory tail, clears it on focus/navigation/app changes, ignores its own generated events, and skips secure input. Its opt-in remains global and is excluded from configuration/profile export.
