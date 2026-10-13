import AppKit
import SwiftUI

private struct HelpCommand: Identifiable {
    let id: String
    let title: String
    let description: String
    let example: String
    let availability: String?
    var aliases = ""
    var searchableText: String { [title, description, example, id, availability ?? "", aliases].joined(separator: " ") }
}

struct HelpView: View {
    @ObservedObject var personalization: PersonalizationStore
    @ObservedObject var quicklinks: QuicklinkStore
    @ObservedObject var commandShortcuts: CommandShortcutStore
    @ObservedObject var appleShortcuts: AppleShortcutsService
    @ObservedObject var appShortcuts: AppShortcutStore
    @ObservedObject var clipboard: ClipboardHistoryStore
    let launcherShortcutLabel: String
    var onReplayOnboarding: () -> Void = {}
    var onExecute: (String) -> Void = { _ in }

    @State private var query = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            TextField(L("Search commands and help"), text: $query)
                .textFieldStyle(.roundedBorder)
                .accessibilityLabel(L("Search commands and help"))
            Toggle(L("Show shortcut hints in the launcher"), isOn: $personalization.settings.showShortcutHints)
            Text("Open the launcher with \(launcherShortcutLabel), then type a command or search for an app, file, or link.")
                .font(.callout).foregroundStyle(.secondary)
            List(filteredCommands) { command in
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: symbol(for: command.id))
                        .frame(width: 24, height: 24)
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(command.title).font(.headline)
                            Spacer()
                            if let shortcut = shortcutLabel(for: command.id) {
                                Text(shortcut).font(.caption.monospaced()).foregroundStyle(.secondary)
                                    .accessibilityLabel("Shortcut: \(shortcut)")
                            }
                        }
                        Text(command.description).font(.callout)
                        Text("Try: \(command.example)").font(.caption).foregroundStyle(.secondary)
                        if let availability = command.availability {
                            Text(availability).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    Button(L("Open")) { onExecute(command.id) }
                        .accessibilityLabel("Open command \(command.title)")
                }
                .padding(.vertical, 3)
            }
            let visibleAppShortcuts = appShortcuts.bindings.filter { binding in
                query.isEmpty
                    || binding.appName.localizedCaseInsensitiveContains(query)
                    || binding.label.localizedCaseInsensitiveContains(query)
            }
            if !visibleAppShortcuts.isEmpty {
                GroupBox(L("Current app shortcuts")) {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(visibleAppShortcuts) { binding in
                            HStack {
                                Text(binding.appName)
                                Spacer()
                                Text(binding.label).font(.caption.monospaced())
                                    .accessibilityLabel("Shortcut: \(binding.label)")
                            }
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 4)
                }
            }
            HStack {
                Button(L("Replay Welcome")) { onReplayOnboarding() }
                    .accessibilityHint(L("Review login, clipboard, and optional permission choices."))
                Spacer()
                Text(L("Help works offline and updates as you add links and shortcuts."))
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(12)
    }

    private var commands: [HelpCommand] {
        let builtIns = LauncherCommand.allCases.map { command in
            let availability: String?
            switch command {
            case .clipboard:
                availability = personalizationSettingsClipboardStatus
            case .quicklinks:
                availability = "\(quicklinks.links.count) saved quicklink\(quicklinks.links.count == 1 ? "" : "s")."
            case .webSearch:
                availability = "\(quicklinks.webSearches.count) web search template\(quicklinks.webSearches.count == 1 ? "" : "s") configured."
            case .appleShortcuts:
                availability = appleShortcuts.selected.isEmpty ? "No Apple Shortcuts selected yet." : "\(appleShortcuts.selected.count) Apple Shortcut\(appleShortcuts.selected.count == 1 ? "" : "s") available."
            case .shutdown:
                availability = "macOS asks you to confirm before shutting down."
            default:
                availability = nil
            }
            return HelpCommand(
                id: command.id,
                title: command.title,
                description: description(for: command),
                example: example(for: command),
                availability: availability,
                aliases: command.aliases.joined(separator: " ")
            )
        }
        let links = quicklinks.links.map { link in
            HelpCommand(
                id: "link:\(link.id)",
                title: link.name,
                description: link.keyword.isEmpty ? "Open the saved link by its name." : "Type its keyword and an optional search argument.",
                example: link.keyword.isEmpty ? link.name : "\(link.keyword) kebloop",
                availability: link.keyword.isEmpty ? nil : "Keyword: \(link.keyword)"
            )
        }
        let shortcuts = appleShortcuts.selected.map { shortcut in
            HelpCommand(
                id: "apple:\(shortcut.id)",
                title: shortcut.name,
                description: "Run this Apple Shortcut from Kebloop. You can pass the current query as text input.",
                example: "\(shortcut.name) or assign it a shortcut key",
                availability: "Available on this Mac while the shortcut remains installed."
            )
        }
        return builtIns + links + shortcuts
    }

    private var filteredCommands: [HelpCommand] {
        let text = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return commands }
        return commands.filter {
            $0.searchableText.localizedCaseInsensitiveContains(text)
                || (shortcutLabel(for: $0.id)?.localizedCaseInsensitiveContains(text) ?? false)
        }
    }

    private var personalizationSettingsClipboardStatus: String {
        "Clipboard recording is \(clipboard.enabled ? "enabled" : "disabled"); saved text stays on this Mac."
    }

    private func shortcutLabel(for id: String) -> String? {
        if let binding = commandShortcuts.bindings.first(where: { $0.id == id }) { return binding.label }
        return nil
    }

    private func symbol(for id: String) -> String {
        if let command = LauncherCommand(rawValue: id) { return command.symbol }
        if id.hasPrefix("apple:") { return "square.stack.3d.up" }
        if id.hasPrefix("link:") { return "link" }
        return "command"
    }

    private func description(for command: LauncherCommand) -> String {
        switch command {
        case .newNote: L("Create a Markdown note immediately and start editing. The launcher also supports ⌘N.")
        case .windowSearch: L("Search open window titles and bring a document forward. Requires Accessibility permission.")
        case .menuSearch: L("Search the previous application’s menu and execute an enabled command. Requires Accessibility permission.")
        case .calendar: L("Choose local calendars explicitly and browse upcoming meetings.")
        case .joinNextMeeting: L("Open the next eligible meeting link after an explicit action.")
        case .selectedText: L("Read selected text, choose processing, and preview before copying or replacing.")
        case .notes: L("Create, search, and autosave local Markdown notes in a floating window.")
        case .dictionary: L("Look up a word using the macOS Dictionary application.")
        case .dictation: L("Record speech on device with consent and preview before inserting.")
        case .raycastImport: L("Preview supported Raycast settings and select categories to import.")
        case .raycastExtensions: L("Review and run the supported subset of Raycast commands.")
        case .windowLayouts: L("Save and restore arrangements with reports for missing windows and displays.")

        case .snippets: L("Fill placeholders, preview, and copy reusable text.")
        case .scripts: L("Run reviewed local scripts with bounded output and cancellation.")
        case .extensions: L("Browse results and actions from enabled local extensions.")
        case .clipboard: L("Browse and paste saved clipboard items. Command-Return copies; Command-Shift-Return pastes plain text.")
        case .quicklinks: L("Open saved websites, files, and folders. Use a keyword with text to pass it as an argument.")
        case .emoji: L("Search emoji by name or category. Return pastes; Command-Return copies.")
        case .settings: L("Open Kebloop settings, including search, appearance, shortcuts, and profiles.")
        case .shutdown: L("Open the macOS shutdown confirmation.")
        case .lock, .sleep, .restart, .logOut, .toggleMute, .toggleAppearance: L("Control your Mac. Restart and log out show macOS confirmation.")
        case .leftHalf, .rightHalf, .maximizeWindow, .centerWindow, .restoreWindow, .nextDisplay, .topHalf, .bottomHalf, .topLeftQuarter, .topRightQuarter, .bottomLeftQuarter, .bottomRightQuarter, .leftThird, .centerThird, .rightThird, .leftTwoThirds, .rightTwoThirds, .previousDisplay: L("Arrange the previous application's window. Requires Accessibility permission.")
        case .calculationHistory: L("Browse, copy, and clear calculations you accepted.")
        case .browse: L("Search the available built-in commands, quicklinks, and selected Apple Shortcuts.")
        case .help: L("Read command examples and see which features and shortcuts are currently available.")
        case .onboarding: L("Review the first-run choices for login startup, clipboard history, and optional Siri access.")
        case .profiles: L("Switch between saved work and personal settings profiles.")
        case .webSearch: L("Send your query to a configured web search template after you choose the result.")
        case .appleShortcuts: L("Browse the Apple Shortcuts you selected in Settings.")
        }
    }

    private func example(for command: LauncherCommand) -> String {
        switch command {
        case .newNote, .windowSearch, .menuSearch, .calendar, .joinNextMeeting, .selectedText, .notes, .dictionary, .dictation, .raycastImport, .raycastExtensions, .windowLayouts: command.aliases.first ?? command.rawValue

        case .snippets: L("snippets")
        case .scripts: L("scripts")
        case .extensions: L("extensions")
        case .clipboard: "clipboard"
        case .quicklinks: L("docs or gh kebloop")
        case .emoji: L("emoji smile")
        case .settings: "settings"
        case .shutdown: "shutdown"
        case .lock, .sleep, .restart, .logOut, .toggleMute, .toggleAppearance, .leftHalf, .rightHalf, .maximizeWindow, .centerWindow, .restoreWindow, .nextDisplay, .topHalf, .bottomHalf, .topLeftQuarter, .topRightQuarter, .bottomLeftQuarter, .bottomRightQuarter, .leftThird, .centerThird, .rightThird, .leftTwoThirds, .rightTwoThirds, .previousDisplay, .calculationHistory: command.aliases.first ?? command.rawValue
        case .browse: "commands"
        case .help: "help"
        case .onboarding: "welcome"
        case .profiles: "profiles"
        case .webSearch: L("web Warsaw cafés")
        case .appleShortcuts: "shortcuts"
        }
    }
}

struct OnboardingView: View {
    @ObservedObject var preferences: Preferences
    @ObservedObject var clipboard: ClipboardHistoryStore
    var onFinish: () -> Void = {}

    @AppStorage("onboardingDismissed") private var onboardingDismissed = false
    @State private var siriTrusted = false

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Label(L("Welcome to Kebloop"), systemImage: "sparkles")
                .font(.largeTitle.weight(.semibold))
            Text(L("Find apps, files, links, emoji, and saved text from one keyboard-first launcher."))
                .font(.title3).foregroundStyle(.secondary)

            GroupBox(L("Choose the basics")) {
                VStack(alignment: .leading, spacing: 12) {
                    Toggle(L("Launch Kebloop when I log in"), isOn: Binding(
                        get: { preferences.loginEnabled },
                        set: { preferences.setLogin($0) }
                    ))
                    if let error = preferences.loginError {
                        Text(error).font(.caption).foregroundStyle(.red)
                    }
                    Toggle(L("Record copied text in clipboard history"), isOn: $clipboard.enabled)
                    Text(L("Clipboard text stays on this Mac. Kebloop skips password-marked, concealed, transient, and oversized items. You can change this choice any time in Settings → Clipboard."))
                        .font(.caption).foregroundStyle(.secondary)
                }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 4)
            }

            GroupBox(L("Optional Siri permission")) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(L("Siri text submission can fill and send your query directly. This uses macOS Accessibility access. ChatGPT, Claude, clipboard, and local search do not need it."))
                        .font(.callout)
                    if siriTrusted {
                        Label(L("Accessibility access is already enabled."), systemImage: "checkmark.circle")
                            .font(.caption).foregroundStyle(.secondary)
                    } else {
                        Button(L("Set Up Siri Access…")) {
                            _ = SiriPromptSender.requestPermission()
                            if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") {
                                NSWorkspace.shared.open(url)
                            }
                            siriTrusted = SiriPromptSender.isTrusted
                        }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 4)
            }

            GroupBox(L("Try these commands")) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(L("clipboard — find and copy saved text"))
                    Text(L("emoji smile — search and copy an emoji"))
                    Text(L("help — browse all commands and shortcuts"))
                    Text(L("⌘K on a result — open its available actions"))
                }.font(.callout).frame(maxWidth: .infinity, alignment: .leading).padding(.top, 4)
            }

            Spacer(minLength: 0)
            HStack {
                Button(L("Skip for now")) { finish() }
                Spacer()
                Button(L("Get Started")) { finish() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(minWidth: 600, minHeight: 540)
        .onAppear { siriTrusted = SiriPromptSender.isTrusted }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            siriTrusted = SiriPromptSender.isTrusted
        }
        .onDisappear { if !onboardingDismissed { finish() } }
    }

    private func finish() {
        onboardingDismissed = true
        onFinish()
    }
}
