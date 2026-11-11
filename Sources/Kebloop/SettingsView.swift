import AppKit
import Carbon
import SwiftUI
import UniformTypeIdentifiers

struct SettingsView: View {
    @ObservedObject var preferences: Preferences
    @ObservedObject var index: ApplicationIndex
    @ObservedObject var quicklinks: QuicklinkStore
    @ObservedObject var shortcuts: AppShortcutStore
    @ObservedObject var aliases: AppAliasStore
    @ObservedObject var clipboard: ClipboardHistoryStore
    @ObservedObject var ai: AIHandoffService
    @ObservedObject var personalization: PersonalizationStore
    @ObservedObject var configuration: ConfigurationStore
    @ObservedObject var commandShortcuts: CommandShortcutStore
    @ObservedObject var appleShortcuts: AppleShortcutsService
    @ObservedObject var snippets: SnippetStore
    @ObservedObject var scripts: ScriptCommandStore
    @ObservedObject var extensions: ExtensionStore
    @ObservedObject var newFiles: NewFileStore
    var onReplayOnboarding: () -> Void = {}
    var onExecute: (String) -> Void = { _ in }

    @State private var excludedClipboardApp = ""
    @State private var editingID: String?
    @State private var linkName = ""
    @State private var linkTarget = ""
    @State private var linkKeyword = ""
    @State private var linkError: String?
    @State private var webSearchName = "Web Search"
    @State private var webSearchKeyword = "web"
    @State private var webSearchTarget = "https://duckduckgo.com/?q={query}"
    @State private var webSearchError: String?
    @State private var editingWebSearchID: String?
    @State private var appleShortcutInput = ""
    @State private var appleShortcutFeedback: String?
    @State private var appQuery = ""
    @State private var recordingApp: IndexedApplication?
    @State private var recordingCommandID: String?
    @State private var recordingLauncherShortcut = false
    @State private var clearConfirmation = false
    @State private var resetSearchConfirmation = false
    @State private var folderFeedback: String?
    @State private var hiddenAppFeedback: String?
    @State private var pathDraft = ""
    @State private var pendingPreview: ConfigurationPreview?
    @State private var configurationFeedback: String?
    @State private var profileFeedback: String?
    @State private var newProfileName = ""
    @State private var profileNameDrafts: [String: String] = [:]
    @State private var selectedSection: SettingsSection = .general
    @AccessibilityFocusState private var shortcutButtonFocus: String?

    var body: some View {
        NavigationSplitView {
            List(selection: sectionSelection) {
                ForEach(SettingsSectionGroup.allCases) { group in
                    Section(group.title) {
                        ForEach(group.sections) { section in
                            Label(section.title, systemImage: section.symbol)
                                .tag(section)
                        }
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationTitle(L("Kebloop"))
            .navigationSplitViewColumnWidth(min: 190, ideal: 220, max: 280)
        } detail: {
            Group {
                selectedSettingsPage
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
            .navigationTitle(selectedSection.title)
            .formStyle(.grouped)
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        selectedSection = .profiles
                    } label: {
                        Label(configuration.activeProfileName, systemImage: "person.crop.circle")
                    }
                    .help(L("Open profile settings"))
                }
            }
        }
        .navigationSplitViewStyle(.balanced)
        .frame(minWidth: 820, minHeight: 580)
        .preferredColorScheme(settingsColorScheme)
        .onReceive(NotificationCenter.default.publisher(for: Notification.Name("KebloopSettingsTab"))) { notification in
            guard let tab = notification.object as? String else { return }
            if let section = SettingsSection(rawValue: tab) { selectedSection = section }
            else if tab == "webSearch" { selectedSection = .quicklinks }
        }
        .onReceive(NotificationCenter.default.publisher(for: Notification.Name("KebloopEditQuicklink"))) { notification in
            guard let link = notification.object as? Quicklink else { return }
            selectedSection = .quicklinks
            editingID = link.id
            linkName = link.name
            linkTarget = link.target
            linkKeyword = link.keyword
            linkError = nil
        }
        .onDisappear { stopAllRecordings() }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)) { _ in
            if recordingApp != nil || recordingCommandID != nil { stopAllRecordings() }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didResignActiveNotification)) { _ in
            if recordingApp != nil || recordingCommandID != nil { stopAllRecordings() }
        }
        .alert("Reset search settings?", isPresented: $resetSearchConfirmation) {
            Button(L("Cancel"), role: .cancel) {}
            Button(L("Reset Search Settings"), role: .destructive) { resetSearchSettings() }
        } message: {
            Text(L("Search categories, folders, exclusions, hidden apps, empty-launcher behavior, and pinned favorites will return to their defaults."))
        }
    }

    private var settingsColorScheme: ColorScheme? {
        switch personalization.settings.appearance {
        case "light": .light
        case "dark": .dark
        default: nil
        }
    }

    private var sectionSelection: Binding<SettingsSection?> {
        Binding(
            get: { selectedSection },
            set: { if let section = $0 { selectedSection = section } }
        )
    }

    @ViewBuilder
    private var selectedSettingsPage: some View {
        switch selectedSection {
        case .newFile: NewFileView(store: newFiles)
        case .scripts: scriptsSettings
        case .extensions: extensionsSettings
        case .snippets: SnippetSettingsView(store: snippets)
        case .general: general
        case .search: searchSettings
        case .appearance: appearanceSettings
        case .quicklinks: links
        case .appShortcuts: appShortcuts
        case .commandShortcuts: commandShortcutSettings
        case .windowManagement: WindowManagementSettingsView(store: commandShortcuts)
        case .appleShortcuts: appleShortcutSettings
        case .profiles: profiles
        case .configuration: configurationSettings
        case .aliases: appAliases
        case .clipboard: clipboardSettings
        case .help:
            HelpView(
                personalization: personalization,
                quicklinks: quicklinks,
                commandShortcuts: commandShortcuts,
                appleShortcuts: appleShortcuts,
                appShortcuts: shortcuts,
                clipboard: clipboard,
                launcherShortcutLabel: preferences.shortcut.label,
                onReplayOnboarding: onReplayOnboarding,
                onExecute: onExecute
            )
        }
    }

    private var scriptsSettings: some View {
        Form {
            Section(L("Script Commands")) {
                ScriptCommandsSettingsView(store: scripts)
            }
        }
        .formStyle(.grouped)
    }

    private var extensionsSettings: some View {
        Form {
            Section(L("Extensions")) {
                ExtensionsSettingsView(store: extensions)
            }
        }
        .formStyle(.grouped)
    }

    private func settingsPage<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        Form(content: content)
            .formStyle(.grouped)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var general: some View {
        settingsPage {
            Section(L("Launcher")) {
                VStack(alignment: .leading, spacing: 12) {
                Picker(L("Open launcher"), selection: $preferences.shortcutID) {
                    ForEach(Shortcut.choices) { Text($0.label).tag($0.id) }
                    if let custom = preferences.customShortcut { Text("Custom · \(custom.label)").tag("custom") }
                }
                .onChange(of: preferences.shortcutID) { _, _ in preferences.saveShortcut() }
                Button(L("Record Custom Shortcut…")) {
                    stopAllRecordings()
                    recordingLauncherShortcut = true
                    shortcuts.suspend()
                    commandShortcuts.suspend()
                }
                if let error = preferences.shortcutError { errorText(error) }
                Text(L("⌘ Space may conflict with Spotlight or Raycast."))
                    .font(.caption).foregroundStyle(.secondary)
                if recordingLauncherShortcut {
                    recorderRow(title: "Press a shortcut for the launcher…", cancel: stopAllRecordings) { keyCode, modifiers, label in
                        preferences.saveCustomShortcut(keyCode: keyCode, modifiers: modifiers, label: label)
                        stopAllRecordings()
                    }
                    if let error = preferences.shortcutError { errorText(error) }
                }
                Toggle(L("Launch at login"), isOn: Binding(get: { preferences.loginEnabled }, set: { preferences.setLogin($0) }))
                if let error = preferences.loginError { errorText(error) }
                }
            }

            Section(L("AI handoff")) {
                VStack(alignment: .leading, spacing: 10) {
                Picker(L("AI provider"), selection: $ai.provider) {
                    ForEach(AIProvider.allCases) { provider in Text(provider.settingsTitle).tag(provider) }
                }
                Text(ai.provider.explanation).font(.caption).foregroundStyle(.secondary)
                Text(L("Type a query in the launcher, then press Tab to open the selected provider. Local Assistant opens an inline chat; web tools send requests online."))
                    .font(.caption).foregroundStyle(.secondary)
                }
            }
            Section(L("Applications")) {
                VStack(alignment: .leading, spacing: 10) {
                Button(L("Refresh Applications")) { index.refresh() }.disabled(index.isRefreshing)
                Button(L("Welcome and Help")) { onReplayOnboarding() }
                    .accessibilityHint(L("Opens the first-run walkthrough and command guide."))
                }
            }
        }
    }

    private var searchSettings: some View {
        Form {
                Section(L("Search categories")) {
                    VStack(alignment: .leading, spacing: 8) {
                        Toggle(L("Applications"), isOn: $personalization.settings.appsEnabled)
                        Toggle(L("Files and folders"), isOn: $personalization.settings.filesEnabled)
                        Toggle(L("Quicklinks and web searches"), isOn: $personalization.settings.linksEnabled)
                        Toggle(L("Built-in commands"), isOn: $personalization.settings.commandsEnabled)
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 4)
                }

                Section(L("Searchable folders")) {
                    VStack(alignment: .leading, spacing: 10) {
                        Text(L("File results are limited to these folders. You can select more than one."))
                            .font(.caption).foregroundStyle(.secondary)
                        if personalization.settings.folders.isEmpty {
                            Text(L("No folders selected. File search is disabled until you add one.")).font(.callout).foregroundStyle(.secondary)
                        }
                        ForEach(personalization.settings.folders, id: \.self) { path in
                            pathRow(path, unavailableMessage: "This folder is unavailable.") {
                                personalization.settings.folders.removeAll { $0 == path }
                            }
                        }
                        HStack {
                            Button(L("Choose Folders…"), systemImage: "folder.badge.plus") { chooseFolders() }
                            Spacer()
                        }
                        if let folderFeedback { Text(folderFeedback).font(.caption).foregroundStyle(.secondary) }
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 4)
                }

                Section(L("Exclude paths")) {
                    VStack(alignment: .leading, spacing: 10) {
                        Text(L("Excluded folders stay out of search even when they sit inside a searchable folder."))
                            .font(.caption).foregroundStyle(.secondary)
                        ForEach(personalization.settings.exclusions, id: \.self) { path in
                            pathRow(path, unavailableMessage: "This excluded path is unavailable.") {
                                personalization.settings.exclusions.removeAll { $0 == path }
                            }
                        }
                        HStack(spacing: 8) {
                            TextField(L("/path/to/exclude or ~/Folder"), text: $pathDraft)
                                .onSubmit(addExclusion)
                            Button(L("Add")) { addExclusion() }
                                .disabled(pathDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                            Button(L("Choose…")) { chooseExclusion() }
                                .accessibilityLabel(L("Choose a folder to exclude"))
                        }
                        if let folderFeedback { Text(folderFeedback).font(.caption).foregroundStyle(.secondary) }
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 4)
                }

                Section(L("Hidden applications")) {
                    VStack(alignment: .leading, spacing: 10) {
                        Text(L("Hide selected apps from launcher results. This does not remove app shortcuts or aliases."))
                            .font(.caption).foregroundStyle(.secondary)
                        ForEach(personalization.settings.hiddenApps, id: \.self) { path in
                            pathRow(path, unavailableMessage: "This application is unavailable.") {
                                personalization.settings.hiddenApps.removeAll { $0 == path }
                            }
                        }
                        HStack { Button(L("Choose Applications…"), systemImage: "app.badge.plus") { chooseHiddenApps() }; Spacer() }
                        if let hiddenAppFeedback { Text(hiddenAppFeedback).font(.caption).foregroundStyle(.secondary) }
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 4)
                }

                Section(L("Empty launcher")) {
                    Picker(L("Show when search is empty"), selection: $personalization.settings.emptyMode) {
                        Text(L("Frequent apps")).tag("frequent")
                        Text(L("Recent apps")).tag("recent")
                        Text(L("Pinned favorites")).tag("favorites")
                    }
                    .pickerStyle(.segmented)
                    .accessibilityLabel(L("Empty launcher results"))
                }

                Section(L("Pinned favorites")) {
                    VStack(alignment: .leading, spacing: 8) {
                        if personalization.pins.isEmpty {
                            Text(L("Pin supported results with ⌘K in the launcher. Favorites appear here and can be shown when search is empty."))
                                .font(.callout).foregroundStyle(.secondary)
                        } else {
                            ForEach(personalization.pins) { pin in
                                HStack(spacing: 8) {
                                    Image(systemName: pinSymbol(pin.kind)).foregroundStyle(.secondary)
                                        .accessibilityHidden(true)
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(pin.name)
                                        Text(pin.path).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                    }
                                    Spacer()
                                    Button(role: .destructive) { personalization.removePin(id: pin.id) } label: {
                                        Image(systemName: "pin.slash")
                                    }
                                    .buttonStyle(.borderless)
                                    .accessibilityLabel("Remove \(pin.name) from pinned favorites")
                                }
                            }
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 4)
                }

                Section {
                    HStack {
                    Spacer()
                    Button(L("Reset Search Settings…"), role: .destructive) { resetSearchConfirmation = true }
                    }
                }
        }
        .formStyle(.grouped)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var appearanceSettings: some View {
        settingsPage {
            Section(L("Appearance")) {
                VStack(alignment: .leading, spacing: 12) {
                Picker(L("Color scheme"), selection: $personalization.settings.appearance) {
                    Text(L("System")).tag("system")
                    Text(L("Light")).tag("light")
                    Text(L("Dark")).tag("dark")
                }
                Slider(value: $personalization.settings.textSize, in: 11...24, step: 1) {
                    Text(L("Text size"))
                } minimumValueLabel: { Text(L("A")) } maximumValueLabel: { Text(L("A")).font(.title) }
                Text("\(Int(personalization.settings.textSize)) pt sample text")
                    .font(.system(size: personalization.settings.textSize))
                    .frame(maxWidth: .infinity, alignment: .leading)
                Slider(value: $personalization.settings.panelWidth, in: 420...1000, step: 20) {
                    Text(L("Panel width"))
                } minimumValueLabel: { Text(L("Narrow")) } maximumValueLabel: { Text(L("Wide")) }
                Text("\(Int(personalization.settings.panelWidth)) points")
                    .font(.caption).foregroundStyle(.secondary)
                Toggle(L("Compact rows"), isOn: $personalization.settings.compact)
                Toggle(L("Use translucent panel"), isOn: $personalization.settings.transparency)
                Text(L("macOS accessibility display settings take priority over reduced transparency and motion effects."))
                    .font(.caption).foregroundStyle(.secondary)
                }
            }

            Section(L("Panel placement")) {
                VStack(alignment: .leading, spacing: 10) {
                Picker(L("Place launcher on"), selection: $personalization.settings.placement) {
                    Text(L("Pointer display")).tag("pointer")
                    Text(L("Active window display")).tag("active")
                    Text(L("Selected display")).tag("display")
                }
                if personalization.settings.placement == "display" {
                    Picker(L("Display"), selection: $personalization.settings.displayID) {
                        ForEach(availableDisplays) { display in Text(display.name).tag(display.id) }
                    }
                    if !personalization.settings.displayID.isEmpty && !availableDisplays.contains(where: { $0.id == personalization.settings.displayID }) {
                        Text(L("The selected display is unavailable. Kebloop will use the pointer display until it returns."))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if availableDisplays.isEmpty { Text(L("No display information is available.")).font(.caption).foregroundStyle(.secondary) }
                }
                }
            }
            Section {
                Button(L("Reset Appearance to Defaults")) { resetAppearanceSettings() }
            }
        }
    }

    private var links: some View {
        settingsPage {
            Section(L("Quicklinks")) {
                VStack(alignment: .leading, spacing: 12) {
                    Text(L("Open websites, files, and folders by name. A keyword can also run a link with an argument, such as “gh kebloop”."))
                        .font(.callout).foregroundStyle(.secondary)
                    if quicklinks.links.isEmpty {
                        ContentUnavailableView(L("No quicklinks yet"), systemImage: "link", description: Text(L("Add a website, file, or folder below.")))
                            .frame(maxWidth: .infinity, minHeight: 96)
                    } else {
                        VStack(spacing: 0) {
                            ForEach(quicklinks.links) { link in
                                HStack(spacing: 10) {
                                    Image(systemName: "link").foregroundStyle(.secondary).accessibilityHidden(true)
                                    VStack(alignment: .leading, spacing: 3) {
                                        Text(link.name).font(.callout.weight(.medium))
                                        Text([link.keyword.isEmpty ? nil : "\(link.keyword) …", link.target].compactMap { $0 }.joined(separator: "  ·  "))
                                            .font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                                    }
                                    Spacer(minLength: 4)
                                    Button(L("Edit")) {
                                        editingID = link.id; linkName = link.name; linkTarget = link.target; linkKeyword = link.keyword; linkError = nil
                                    }
                                    Button(role: .destructive) { quicklinks.remove(id: link.id); if editingID == link.id { resetLink() } } label: {
                                        Image(systemName: "trash")
                                    }
                                    .buttonStyle(.borderless)
                                    .accessibilityLabel("Remove quicklink \(link.name)")
                                }
                                .padding(.vertical, 8)
                                if link.id != quicklinks.links.last?.id { Divider() }
                            }
                        }
                    }

                    Divider()
                    Text(editingID == nil ? "Add a quicklink" : "Edit quicklink")
                        .font(.subheadline.weight(.semibold))
                    TextField(L("Name"), text: $linkName)
                    TextField(L("Keyword (optional)"), text: $linkKeyword)
                    TextField(L("Website URL, file path, or folder path"), text: $linkTarget)
                    if !linkTarget.isEmpty, linkTarget.contains("{query}") {
                        Text("Preview: \(Quicklink.destination(linkTarget, argument: "A space & tea")?.absoluteString ?? "This template is invalid.")")
                            .font(.caption).foregroundStyle(.secondary).lineLimit(2).textSelection(.enabled)
                    }
                    if let linkError { errorText(linkError) }
                    HStack {
                        Button(editingID == nil ? "Add Quicklink" : "Save Quicklink") {
                            linkError = quicklinks.save(id: editingID, name: linkName, target: linkTarget, keyword: linkKeyword)
                            if linkError == nil { resetLink() }
                        }
                        .disabled(linkName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || linkTarget.isEmpty)
                        if editingID != nil { Button(L("Cancel")) { resetLink() } }
                        Spacer()
                    }
                }
            }

            Section(L("Web search templates")) {
                VStack(alignment: .leading, spacing: 12) {
                    Text(L("Use {query} where the encoded search text should go. For example, type “web cats” in the launcher."))
                        .font(.callout).foregroundStyle(.secondary)
                    if quicklinks.webSearches.isEmpty {
                        ContentUnavailableView(L("No web searches yet"), systemImage: "magnifyingglass", description: Text(L("Add a search template with {query} in its URL.")))
                            .frame(maxWidth: .infinity, minHeight: 96)
                    } else {
                        VStack(spacing: 0) {
                            ForEach(quicklinks.webSearches) { search in
                                HStack(spacing: 10) {
                                    Image(systemName: "magnifyingglass").foregroundStyle(.secondary).accessibilityHidden(true)
                                    VStack(alignment: .leading, spacing: 3) {
                                        Text("\(search.name)  ·  \(search.keyword)").font(.callout.weight(.medium))
                                        Text(search.target).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                                    }
                                    Spacer(minLength: 4)
                                    Button(L("Edit")) {
                                        editingWebSearchID = search.id
                                        webSearchName = search.name; webSearchKeyword = search.keyword; webSearchTarget = search.target; webSearchError = nil
                                    }
                                    Button(role: .destructive) { quicklinks.removeWebSearch(id: search.id) } label: {
                                        Image(systemName: "trash")
                                    }
                                    .buttonStyle(.borderless)
                                    .accessibilityLabel("Remove web search \(search.name)")
                                }
                                .padding(.vertical, 8)
                                if search.id != quicklinks.webSearches.last?.id { Divider() }
                            }
                        }
                    }
                    Divider()
                    Text(editingWebSearchID == nil ? "Add a web search" : "Edit web search")
                        .font(.subheadline.weight(.semibold))
                    TextField(L("Name"), text: $webSearchName)
                    TextField(L("Keyword"), text: $webSearchKeyword)
                    TextField(L("https://example.com/?q={query}"), text: $webSearchTarget)
                    if !webSearchTarget.isEmpty {
                        let preview = WebSearchTemplate(id: "preview", name: webSearchName, keyword: webSearchKeyword, target: webSearchTarget).resolvedURL(argument: "Warsaw cafés & tea")
                        Text("Preview: \(preview?.absoluteString ?? "Use a valid HTTP or HTTPS URL with {query}.")")
                            .font(.caption).foregroundStyle(.secondary).lineLimit(2).textSelection(.enabled)
                    }
                    if let webSearchError { errorText(webSearchError) }
                    HStack {
                        Button(editingWebSearchID == nil ? "Add Web Search" : "Save Web Search") {
                            webSearchError = quicklinks.saveWebSearch(id: editingWebSearchID, name: webSearchName, keyword: webSearchKeyword, target: webSearchTarget)
                            if webSearchError == nil { editingWebSearchID = nil }
                        }
                        Spacer()
                    }
                }
            }
        }
    }

    private var appShortcuts: some View {
        settingsPage {
            Section(L("Application shortcuts")) {
                VStack(alignment: .leading, spacing: 12) {
                    TextField(L("Search applications"), text: $appQuery)
                        .accessibilityLabel(L("Search applications for shortcuts"))
                    Text(L("Open an app from anywhere with Command, Option, or Control plus a key."))
                        .font(.callout).foregroundStyle(.secondary)
                    if filteredApplications.isEmpty {
                        ContentUnavailableView(L("No matching applications"), systemImage: "app.dashed", description: Text(L("Try a different search or refresh the application index in General settings.")))
                            .frame(maxWidth: .infinity, minHeight: 120)
                    } else {
                        VStack(spacing: 0) {
                            ForEach(filteredApplications) { app in
                                let binding = shortcuts.bindings.first { $0.appPath == app.path }
                                HStack(spacing: 10) {
                                    appIcon(app)
                                    VStack(alignment: .leading, spacing: 3) {
                                        Text(app.name).font(.callout.weight(.medium))
                                        if let error = shortcuts.errors[app.path] { errorText(error) }
                                    }
                                    Spacer(minLength: 4)
                                    Button(binding?.label ?? "Record Shortcut") {
                                        startAppRecording(app)
                                    }
                                    .accessibilityFocused($shortcutButtonFocus, equals: "app:\(app.path)")
                                    if let binding {
                                        Button(role: .destructive) { shortcuts.remove(id: binding.id) } label: { Image(systemName: "xmark.circle") }
                                            .buttonStyle(.borderless)
                                            .accessibilityLabel("Remove shortcut for \(app.name)")
                                    }
                                }
                                .padding(.vertical, 9)
                                if app.id != filteredApplications.last?.id { Divider() }
                            }
                        }
                    }
                    if let app = recordingApp {
                        recorderRow(title: "Press a shortcut for \(app.name)…", cancel: stopAllRecordings) { keyCode, modifiers, label in
                            if shortcuts.assign(app: app, keyCode: keyCode, modifiers: modifiers, label: label) {
                                stopAllRecordings()
                                shortcutButtonFocus = "app:\(app.path)"
                            }
                        }
                        if let conflict = shortcuts.recorderConflict { errorText(conflict) }
                    }
                }
            }
        }
        .onDisappear { stopAppRecording() }
    }

    private var commandShortcutSettings: some View {
        settingsPage {
            Section(L("Command shortcuts")) {
                VStack(alignment: .leading, spacing: 12) {
                    Text(L("Assign a global shortcut to a built-in command, quicklink, or selected Apple Shortcut. The same recorder and conflict checks apply to apps and commands."))
                        .font(.callout).foregroundStyle(.secondary)
                    VStack(spacing: 0) {
                        ForEach(commandShortcutTargets) { command in
                            let binding = commandShortcuts.bindings.first { $0.id == command.id }
                            HStack(spacing: 10) {
                                Image(systemName: command.symbol).frame(width: 24).foregroundStyle(.secondary).accessibilityHidden(true)
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(command.title).font(.callout.weight(.medium))
                                    if let error = commandShortcuts.errors[command.id] { errorText(error) }
                                }
                                Spacer(minLength: 4)
                                Button(binding?.label ?? "Record Shortcut") {
                                    recordingCommandID = command.id
                                    commandShortcuts.recorderConflict = nil
                                    commandShortcuts.suspend()
                                    DispatchQueue.main.async { shortcutButtonFocus = "command:\(command.id)" }
                                }
                                .accessibilityFocused($shortcutButtonFocus, equals: "command:\(command.id)")
                                if let binding {
                                    Button(role: .destructive) { commandShortcuts.remove(id: binding.id) } label: { Image(systemName: "xmark.circle") }
                                        .buttonStyle(.borderless)
                                        .accessibilityLabel("Remove shortcut for \(command.title)")
                                }
                            }
                            .padding(.vertical, 8)
                            if command.id != commandShortcutTargets.last?.id { Divider() }
                        }
                    }
                    if let commandID = recordingCommandID, let command = commandShortcutTargets.first(where: { $0.id == commandID }) {
                        recorderRow(title: "Press a shortcut for \(command.title)…", cancel: stopAllRecordings) { keyCode, modifiers, label in
                            if commandShortcuts.assign(commandID: command.id, keyCode: keyCode, modifiers: modifiers, label: label) {
                                stopAllRecordings()
                                shortcutButtonFocus = "command:\(command.id)"
                            }
                        }
                        if let conflict = commandShortcuts.recorderConflict { errorText(conflict) }
                    }
                }
            }
        }
        .onDisappear { stopCommandRecording() }
    }

    private var appleShortcutSettings: some View {
        settingsPage {
            Section(L("Available Apple Shortcuts")) {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(alignment: .top) {
                        Text(L("Choose shortcuts to make available as launcher commands. macOS may ask permission when you run one."))
                            .font(.callout).foregroundStyle(.secondary)
                        Spacer(minLength: 8)
                        Button(L("Refresh"), systemImage: "arrow.clockwise") { appleShortcuts.refresh() }
                            .disabled(appleShortcuts.isLoading)
                            .accessibilityLabel(L("Refresh available Apple Shortcuts"))
                    }
                    if appleShortcuts.isLoading { ProgressView("Loading Shortcuts…").frame(maxWidth: .infinity, alignment: .leading) }
                    if let error = appleShortcuts.error { errorText(error) }
                    if appleShortcuts.available.isEmpty && !appleShortcuts.isLoading {
                        ContentUnavailableView(L("No Shortcuts found"), systemImage: "square.stack.3d.up", description: Text(L("Create shortcuts in Apple's Shortcuts app, then refresh this list.")))
                            .frame(maxWidth: .infinity, minHeight: 140)
                    } else {
                        VStack(spacing: 0) {
                            ForEach(appleShortcuts.available) { shortcut in
                                let selected = appleShortcuts.selected.contains(where: { $0.id == shortcut.id })
                                HStack(spacing: 10) {
                                    Image(systemName: "square.stack.3d.up").foregroundStyle(.secondary).accessibilityHidden(true)
                                    VStack(alignment: .leading, spacing: 3) {
                                        Text(shortcut.name).font(.callout.weight(.medium))
                                        if let error = appleShortcuts.errors[shortcut.id] { errorText(error) }
                                    }
                                    Spacer(minLength: 4)
                                    Button(selected ? "Remove" : "Add") {
                                        if selected { appleShortcuts.remove(id: shortcut.id) }
                                        else { appleShortcuts.select(shortcut) }
                                    }
                                    .accessibilityLabel(selected ? "Remove \(shortcut.name) from Kebloop" : "Add \(shortcut.name) to Kebloop")
                                }
                                .padding(.vertical, 8)
                                if shortcut.id != appleShortcuts.available.last?.id { Divider() }
                            }
                        }
                    }
                }
            }
            if !appleShortcuts.selected.isEmpty {
                Section(L("Selected shortcuts")) {
                    VStack(alignment: .leading, spacing: 10) {
                        Text(L("Optional text input is passed to the selected shortcut when you run it here."))
                            .font(.callout).foregroundStyle(.secondary)
                        TextField(L("Optional text input"), text: $appleShortcutInput)
                            .accessibilityLabel(L("Optional text input for Apple Shortcuts"))
                        ForEach(appleShortcuts.selected) { shortcut in
                            HStack(spacing: 10) {
                                Text(shortcut.name).font(.callout.weight(.medium))
                                Spacer(minLength: 4)
                                if appleShortcuts.runningID == shortcut.id {
                                    ProgressView().controlSize(.small).accessibilityLabel("Running \(shortcut.name)")
                                    Button(L("Cancel Run")) { appleShortcuts.cancel() }
                                } else {
                                    Button(L("Run")) {
                                        let input = appleShortcutInput.trimmingCharacters(in: .whitespacesAndNewlines)
                                        appleShortcutFeedback = nil
                                        appleShortcuts.run(id: shortcut.id, input: input.isEmpty ? nil : input) { error in
                                            appleShortcutFeedback = error
                                        }
                                    }
                                    .disabled(appleShortcuts.isRunning)
                                    .accessibilityLabel("Run \(shortcut.name)\(appleShortcutInput.isEmpty ? "" : " with text input")")
                                }
                            }
                        }
                    }
                }
                if let appleShortcutFeedback { errorText(appleShortcutFeedback) }
            }
        }
        .onAppear { appleShortcuts.refresh() }
        .onDisappear { appleShortcuts.cancel() }
    }

    private var profiles: some View {
        settingsPage {
            Section(L("Profiles")) {
                VStack(alignment: .leading, spacing: 12) {
                    Text(L("Profiles keep links, aliases, shortcuts, pins, and search and appearance preferences together. Launch at login and clipboard privacy stay global."))
                        .font(.callout).foregroundStyle(.secondary)
                    VStack(spacing: 0) {
                        ForEach(configuration.profiles) { profile in
                            VStack(alignment: .leading, spacing: 9) {
                                HStack(spacing: 8) {
                                    TextField(L("Profile name"), text: profileNameBinding(profile))
                                        .onSubmit { renameProfile(profile) }
                                    if profile.id == configuration.activeProfileID {
                                        Label(L("Active"), systemImage: "checkmark.circle.fill")
                                            .font(.caption.weight(.medium)).foregroundStyle(Color.accentColor)
                                    } else {
                                        Button(L("Switch")) { performProfileAction { try configuration.switchProfile(id: profile.id) } }
                                    }
                                }
                                HStack(spacing: 8) {
                                    Button(L("Save name")) { renameProfile(profile) }
                                        .disabled((profileNameDrafts[profile.id] ?? profile.name) == profile.name)
                                    Button(L("Duplicate")) {
                                        let name = "\(profile.name) Copy"
                                        performProfileAction { try configuration.duplicateProfile(id: profile.id, name: name) }
                                    }
                                    .accessibilityLabel("Duplicate profile \(profile.name)")
                                    Button(L("Export")) { exportConfiguration(profileID: profile.id) }
                                        .accessibilityLabel("Export profile \(profile.name)")
                                    Spacer(minLength: 2)
                                    Button(role: .destructive) { performProfileAction { try configuration.deleteProfile(id: profile.id) } } label: {
                                        Label(L("Delete"), systemImage: "trash")
                                    }
                                    .disabled(configuration.profiles.count <= 1)
                                    .accessibilityLabel("Delete profile \(profile.name)")
                                }
                                .controlSize(.small)
                            }
                            .padding(.vertical, 10)
                            if profile.id != configuration.profiles.last?.id { Divider() }
                        }
                    }
                    Divider()
                    Text(L("Create a profile")).font(.subheadline.weight(.semibold))
                    HStack(spacing: 8) {
                        TextField(L("New profile name"), text: $newProfileName)
                        Button(L("Create Profile")) {
                            performProfileAction { _ = try configuration.createProfile(name: newProfileName); newProfileName = "" }
                        }
                        .disabled(newProfileName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                    if let profileFeedback { errorText(profileFeedback) }
                }
            }
        }
        .onAppear { syncProfileDrafts() }
        .onChange(of: configuration.profiles.map(\.id)) { _, _ in syncProfileDrafts() }
    }

    private var configurationSettings: some View {
        settingsPage {
            Section(L("Configuration")) {
                VStack(alignment: .leading, spacing: 12) {
                    Text(L("Export aliases, quicklinks, shortcuts, pins, search and appearance settings, and profiles. Clipboard contents, credentials, caches, and usage history are not included."))
                        .font(.callout).foregroundStyle(.secondary)
                    HStack {
                        Button(L("Export Configuration…"), systemImage: "square.and.arrow.up") { exportConfiguration(profileID: nil) }
                        Button(L("Import Configuration…"), systemImage: "square.and.arrow.down") { importConfiguration() }
                        Spacer(minLength: 0)
                    }
                    if let configurationFeedback { errorText(configurationFeedback) }
                }
            }
            if let preview = pendingPreview {
                Section(L("Import preview")) {
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(preview.summary, id: \.self) { summary in Label(summary, systemImage: "checkmark.circle").font(.callout) }
                        if !preview.warnings.isEmpty {
                            Divider()
                            Text(L("Review before applying")).font(.headline)
                            ForEach(preview.warnings, id: \.self) { warning in Text("• \(warning)").font(.callout).foregroundStyle(.secondary) }
                        }
                        Divider()
                        VStack(alignment: .leading, spacing: 8) {
                            Button(L("Merge")) { applyImport(preview, mode: .merge) }
                            Button(L("Replace Existing Settings…"), role: .destructive) { applyImport(preview, mode: .replace) }
                            Button(L("Cancel")) { pendingPreview = nil; configurationFeedback = nil }
                        }
                        Text(L("Merge adds items with new IDs and keeps existing values on conflicts. Replace saves a backup before applying imported settings."))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    private var clipboardSettings: some View {
        settingsPage {
            Section(L("Clipboard history")) {
                VStack(alignment: .leading, spacing: 12) {
                    Toggle(L("Enable clipboard history"), isOn: $clipboard.enabled)
                    Text(L("Copied text is stored locally on this Mac. Disable history to stop collecting new items."))
                        .font(.callout).foregroundStyle(.secondary)
                    Stepper(value: $clipboard.maximumItemCount, in: 1...200) { Text("\(L("Unpinned item limit")): \(clipboard.maximumItemCount)") }
                    Stepper(value: $clipboard.maximumAgeDays, in: 0...3650) { Text("\(L("Retention days (0 = forever)")): \(clipboard.maximumAgeDays)") }
                    Text(L("Pinned items do not expire and are exempt from the count limit (up to 50 pins). Clear History includes pinned items. Attachments are limited to 5 MB per entry and 50 MB total."))
                        .font(.caption).foregroundStyle(.secondary)
                    HStack {
                        Button(L("Pause for 15 minutes")) { clipboard.pauseRecording(for: 900) }
                        Button(L("Pause for 1 hour")) { clipboard.pauseRecording(for: 3600) }
                        Button(L("Resume Recording")) { clipboard.resumeRecording() }.disabled(!clipboard.isPaused)
                    }
                    if let until = clipboard.pauseUntil, clipboard.isPaused { Text("\(L("Paused until")) \(until.formatted())") }
                    Text(L("Source app attribution uses the foreground app when the clipboard is checked and may be inaccurate. Exclusions cannot guarantee that a background app's content is blocked."))
                        .font(.caption).foregroundStyle(.secondary)
                    HStack {
                        TextField(L("Excluded source app bundle ID"), text: $excludedClipboardApp)
                        Button(L("Add")) {
                            let value = excludedClipboardApp.trimmingCharacters(in: .whitespacesAndNewlines)
                            if !value.isEmpty { clipboard.excludedSourceBundleIDs.insert(value); excludedClipboardApp = "" }
                        }
                    }
                    ForEach(clipboard.excludedSourceBundleIDs.sorted(), id: \.self) { identifier in
                        HStack { Text(identifier); Spacer(); Button(L("Remove")) { clipboard.excludedSourceBundleIDs.remove(identifier) } }
                    }
                    Label("\(clipboard.items.count) saved items", systemImage: "doc.on.clipboard")
                        .font(.callout).foregroundStyle(.secondary)
                    Button(L("Clear Clipboard History"), systemImage: "trash", role: .destructive) { clearConfirmation = true }
                        .disabled(clipboard.items.isEmpty)
                        .alert("Clear clipboard history?", isPresented: $clearConfirmation) {
                            Button(L("Cancel"), role: .cancel) {}
                            Button(L("Clear History"), role: .destructive) { clipboard.clear() }
                        } message: { Text(L("All saved clipboard items, including pinned items and attachments, will be removed.")) }
                }
            }
        }
    }

    private var appAliases: some View {
        settingsPage {
            Section(L("Application aliases")) {
                VStack(alignment: .leading, spacing: 12) {
                    TextField(L("Search applications"), text: $appQuery)
                        .accessibilityLabel(L("Search applications for aliases"))
                    Text(L("Give an app a name to search for, such as “terminal” for Ghostty."))
                        .font(.callout).foregroundStyle(.secondary)
                    let matchingApplications = index.applications.filter { app in
                        appQuery.isEmpty
                            || app.name.localizedCaseInsensitiveContains(appQuery)
                            || (aliases.alias(for: app)?.localizedCaseInsensitiveContains(appQuery) ?? false)
                    }
                    if matchingApplications.isEmpty {
                        ContentUnavailableView(L("No matching applications"), systemImage: "app.dashed", description: Text(L("Try another search term.")))
                            .frame(maxWidth: .infinity, minHeight: 100)
                    } else {
                        VStack(spacing: 0) {
                            ForEach(matchingApplications) { app in
                                AppAliasRow(app: app, index: index, aliases: aliases)
                                    .padding(.vertical, 7)
                                if app.id != matchingApplications.last?.id { Divider() }
                            }
                        }
                    }
                }
            }
        }
    }

    private var filteredApplications: [IndexedApplication] {
        index.applications.filter { appQuery.isEmpty || $0.name.localizedCaseInsensitiveContains(appQuery) }
    }

    private var commandShortcutTargets: [CommandShortcutTarget] {
        let builtIns = LauncherCommand.allCases.map { CommandShortcutTarget(id: $0.id, title: $0.title, symbol: $0.symbol) }
        let links = quicklinks.links.map { CommandShortcutTarget(id: "link:\($0.id)", title: $0.name, symbol: "link") }
        let shortcuts = appleShortcuts.selected.map { CommandShortcutTarget(id: "apple:\($0.id)", title: $0.name, symbol: "square.stack.3d.up") }
        let automation = scripts.commands.filter(\.enabled).map { CommandShortcutTarget(id: $0.commandID, title: $0.title, symbol: "terminal") } + extensions.manifests.filter(\.enabled).map { CommandShortcutTarget(id: $0.commandID, title: $0.name, symbol: "puzzlepiece.extension") }
        return automation + builtIns + links + shortcuts + snippets.snippets.map { CommandShortcutTarget(id: "snippet:\($0.id)", title: $0.name, symbol: "text.quote") }
    }

    private var availableDisplays: [DisplayOption] {
        NSScreen.screens.map { screen in
            let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber
            let identifier = number.map { String($0.uint32Value) } ?? screen.localizedName
            return DisplayOption(id: identifier, name: screen.localizedName)
        }
    }

    private func appIcon(_ app: IndexedApplication) -> some View {
        Group {
            if let icon = index.icon(for: app) { Image(nsImage: icon).resizable() }
            else { Image(systemName: "app.fill").resizable().foregroundStyle(.secondary) }
        }
        .frame(width: 24, height: 24)
        .accessibilityHidden(true)
    }

    private func pathRow(_ path: String, unavailableMessage: String, remove: @escaping () -> Void) -> some View {
        HStack(spacing: 8) {
            Image(systemName: FileManager.default.fileExists(atPath: (path as NSString).expandingTildeInPath) ? "folder" : "exclamationmark.folder")
                .foregroundStyle(FileManager.default.fileExists(atPath: (path as NSString).expandingTildeInPath) ? Color.secondary : Color.orange)
                .accessibilityHidden(true)
            Text(path).lineLimit(1).help(path)
            if !FileManager.default.fileExists(atPath: (path as NSString).expandingTildeInPath) {
                Text(unavailableMessage).font(.caption).foregroundStyle(.orange)
            }
            Spacer()
            Button(role: .destructive, action: remove) { Image(systemName: "trash") }
                .buttonStyle(.borderless)
                .accessibilityLabel("Remove \(path)")
        }
    }

    private func chooseFolders() {
        let panel = NSOpenPanel()
        panel.title = L("Choose Searchable Folders")
        panel.prompt = "Add Folders"
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = true
        guard panel.runModal() == .OK else { return }
        let additions = panel.urls.map(\.standardizedFileURL.path).filter { !$0.isEmpty }
        personalization.settings.folders = Array(Set(personalization.settings.folders + additions)).sorted()
        folderFeedback = additions.isEmpty ? "No folders were added." : "Added \(additions.count) folder\(additions.count == 1 ? "" : "s")."
    }

    private func chooseExclusion() {
        let panel = NSOpenPanel()
        panel.title = L("Choose a Folder to Exclude")
        panel.prompt = "Exclude Folder"
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        appendPath(url.standardizedFileURL.path, to: \.exclusions)
    }

    private func chooseHiddenApps() {
        let panel = NSOpenPanel()
        panel.title = L("Choose Applications to Hide")
        panel.prompt = "Hide Applications"
        panel.allowedContentTypes = [.applicationBundle]
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = true
        guard panel.runModal() == .OK else { return }
        let apps = panel.urls.filter { $0.pathExtension == "app" }.map(\.standardizedFileURL.path)
        guard !apps.isEmpty else { hiddenAppFeedback = "Choose one or more .app bundles."; return }
        personalization.settings.hiddenApps = Array(Set(personalization.settings.hiddenApps + apps)).sorted()
        hiddenAppFeedback = "Added \(apps.count) application\(apps.count == 1 ? "" : "s")."
    }

    private func appendPath(_ value: String, to keyPath: WritableKeyPath<PersonalizationSettings, [String]>) {
        let path = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard path.hasPrefix("/") || path.hasPrefix("~/") else { folderFeedback = "Enter an absolute path or a path beginning with ~/."; return }
        var values = personalization.settings[keyPath: keyPath]
        let expanded = (path as NSString).expandingTildeInPath
        guard FileManager.default.fileExists(atPath: expanded) else { folderFeedback = "This path is unavailable. Choose an existing folder or app."; return }
        guard !values.contains(path) else { folderFeedback = "This path is already listed."; return }
        values.append(path)
        personalization.settings[keyPath: keyPath] = values.sorted()
        folderFeedback = "Added \(path)."
        pathDraft = ""
    }

    private func addExclusion() { appendPath(pathDraft, to: \.exclusions) }

    private func resetSearchSettings() {
        let defaults = PersonalizationSettings()
        var settings = personalization.settings
        settings.appsEnabled = defaults.appsEnabled
        settings.filesEnabled = defaults.filesEnabled
        settings.linksEnabled = defaults.linksEnabled
        settings.commandsEnabled = defaults.commandsEnabled
        settings.folders = defaults.folders
        settings.exclusions = defaults.exclusions
        settings.hiddenApps = defaults.hiddenApps
        settings.emptyMode = defaults.emptyMode
        personalization.settings = settings
        for pin in personalization.pins { personalization.removePin(id: pin.id) }
    }

    private func resetAppearanceSettings() {
        let defaults = PersonalizationSettings()
        var settings = personalization.settings
        settings.appearance = defaults.appearance
        settings.textSize = defaults.textSize
        settings.panelWidth = defaults.panelWidth
        settings.compact = defaults.compact
        settings.transparency = defaults.transparency
        settings.placement = defaults.placement
        settings.displayID = defaults.displayID
        personalization.settings = settings
    }

    private func pinSymbol(_ kind: String) -> String {
        switch kind {
        case "application": "app.fill"
        case "file": "doc"
        case "quicklink": "link"
        default: "terminal"
        }
    }

    private func resetLink() { editingID = nil; linkName = ""; linkTarget = ""; linkKeyword = ""; linkError = nil }

    private func startAppRecording(_ app: IndexedApplication) {
        recordingCommandID = nil
        recordingApp = app
        shortcuts.recorderConflict = nil
        shortcuts.suspend()
        DispatchQueue.main.async { shortcutButtonFocus = "app:\(app.path)" }
    }

    private func stopAllRecordings() {
        stopAppRecording()
        stopCommandRecording()
        stopLauncherRecording()
    }

    private func stopAppRecording() {
        guard recordingApp != nil else { return }
        recordingApp = nil
        shortcuts.recorderConflict = nil
        shortcuts.resume()
    }

    private func stopCommandRecording() {
        guard recordingCommandID != nil else { return }
        recordingCommandID = nil
        commandShortcuts.recorderConflict = nil
        commandShortcuts.resume()
    }

    private func stopLauncherRecording() {
        guard recordingLauncherShortcut else { return }
        recordingLauncherShortcut = false
        shortcuts.resume()
        commandShortcuts.resume()
    }

    private func recorderRow(title: String, cancel: @escaping () -> Void, record: @escaping (UInt32, UInt32, String) -> Void) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.callout.weight(.medium))
            HStack(spacing: 10) {
                ShortcutRecorder(record: record, cancel: cancel)
                    .frame(width: 240, height: 30)
                    .accessibilityLabel(L("Shortcut recorder"))
                    .accessibilityHint("\(title) Press Command, Option, or Control with a key. Press Escape to cancel.")
                Spacer(minLength: 0)
                Button(L("Cancel"), action: cancel)
            }
        }
    }

    private func profileNameBinding(_ profile: ConfigurationProfile) -> Binding<String> {
        Binding(get: { profileNameDrafts[profile.id] ?? profile.name }, set: { profileNameDrafts[profile.id] = $0 })
    }

    private func syncProfileDrafts() {
        for profile in configuration.profiles where profileNameDrafts[profile.id] == nil {
            profileNameDrafts[profile.id] = profile.name
        }
        profileNameDrafts = profileNameDrafts.filter { id, _ in configuration.profiles.contains(where: { $0.id == id }) }
    }

    private func renameProfile(_ profile: ConfigurationProfile) {
        let name = profileNameDrafts[profile.id] ?? profile.name
        performProfileAction { try configuration.renameProfile(id: profile.id, name: name) }
        if profileFeedback == nil { profileNameDrafts[profile.id] = name.trimmingCharacters(in: .whitespacesAndNewlines) }
    }

    private func performProfileAction(_ operation: () throws -> Void) {
        do { try operation(); profileFeedback = nil }
        catch { profileFeedback = error.localizedDescription }
    }

    private func exportConfiguration(profileID: String?) {
        do {
            let data = try configuration.exportData(profileID: profileID)
            let panel = NSSavePanel()
            panel.allowedContentTypes = [.json]
            panel.canCreateDirectories = true
            panel.nameFieldStringValue = profileID == nil ? "Kebloop-configuration.json" : "Kebloop-profile.json"
            guard panel.runModal() == .OK, let url = panel.url else { return }
            try data.write(to: url, options: .atomic)
            configurationFeedback = "Configuration exported to \(url.lastPathComponent)."
        } catch { configurationFeedback = error.localizedDescription }
    }

    private func importConfiguration() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.title = L("Import Kebloop Configuration")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            pendingPreview = try configuration.preview(data: Data(contentsOf: url))
            configurationFeedback = "Review the import preview before applying it."
        } catch { pendingPreview = nil; configurationFeedback = error.localizedDescription }
    }

    private func applyImport(_ preview: ConfigurationPreview, mode: ConfigurationImportMode) {
        var backupURL: URL?
        if mode == .replace {
            let panel = NSSavePanel()
            panel.allowedContentTypes = [.json]
            panel.canCreateDirectories = true
            panel.nameFieldStringValue = "Kebloop-configuration-backup.json"
            panel.title = L("Save Configuration Backup Before Replacing")
            panel.prompt = "Save Backup and Replace"
            guard panel.runModal() == .OK, let url = panel.url else { return }
            backupURL = url
        }
        do {
            try configuration.apply(preview: preview, mode: mode, backupURL: backupURL)
            pendingPreview = nil
            configurationFeedback = mode == .replace ? "Configuration replaced. Backup saved." : "Configuration merged."
        } catch { configurationFeedback = error.localizedDescription }
    }

    @ViewBuilder private func errorText(_ message: String) -> some View {
        Text(message).font(.caption).foregroundStyle(.red).accessibilityAddTraits(.updatesFrequently)
    }
}

private enum SettingsSection: String, CaseIterable, Identifiable {
    case newFile
    case scripts, extensions, snippets, general, search, appearance, windowManagement, quicklinks, appShortcuts, commandShortcuts, appleShortcuts, profiles, configuration, aliases, clipboard, help

    var id: String { rawValue }
    var title: String {
        switch self {
        case .newFile: L("New File")
        case .scripts: L("Script Commands")
        case .extensions: L("Extensions")
        case .snippets: L("Snippets")
        case .general: L("General")
        case .search: L("Search")
        case .appearance: L("Appearance")
        case .windowManagement: L("Window Management")
        case .quicklinks: L("Quicklinks")
        case .appShortcuts: L("App Shortcuts")
        case .commandShortcuts: L("Command Shortcuts")
        case .appleShortcuts: L("Apple Shortcuts")
        case .profiles: L("Profiles")
        case .configuration: L("Configuration")
        case .aliases: L("Aliases")
        case .clipboard: L("Clipboard")
        case .help: L("Help")
        }
    }
    var symbol: String {
        switch self {
        case .newFile: "doc.badge.plus"
        case .scripts: "terminal"
        case .extensions: "puzzlepiece.extension"
        case .snippets: "text.quote"
        case .general: "gearshape"
        case .search: "magnifyingglass"
        case .appearance: "paintbrush"
        case .windowManagement: "rectangle.split.2x2"
        case .quicklinks: "link"
        case .appShortcuts: "command"
        case .commandShortcuts: "keyboard"
        case .appleShortcuts: "square.stack.3d.up"
        case .profiles: "person.crop.circle"
        case .configuration: "square.and.arrow.up.on.square"
        case .aliases: "text.magnifyingglass"
        case .clipboard: "doc.on.clipboard"
        case .help: "questionmark.circle"
        }
    }

    var subtitle: String {
        switch self {
        case .newFile: L("Create files in Finder and customize file types and templates.")
        case .scripts: L("Manage reviewed local script commands.")
        case .extensions: L("Manage local extensions and their preferences.")
        case .snippets: L("Create reusable text with explicit placeholders.")
        case .general: L("Set your launcher shortcut, startup behavior, and AI handoff.")
        case .search: L("Choose what Kebloop searches and manage indexed locations.")
        case .appearance: L("Adjust the launcher’s look, size, and display placement.")
        case .windowManagement: L("Configure window layouts and their keyboard shortcuts.")
        case .quicklinks: L("Manage saved links and keyword-based web searches.")
        case .appShortcuts: L("Assign global shortcuts to your applications.")
        case .commandShortcuts: L("Assign global shortcuts to commands and saved actions.")
        case .appleShortcuts: L("Choose and run Apple Shortcuts from Kebloop.")
        case .profiles: L("Keep separate sets of launcher preferences and saved items.")
        case .configuration: L("Preview, import, and export Kebloop settings.")
        case .aliases: L("Give applications additional names for search.")
        case .clipboard: L("Control local clipboard history and saved items.")
        case .help: L("Browse commands, examples, and your current shortcuts.")
        }
    }
}

private enum SettingsSectionGroup: String, CaseIterable, Identifiable {
    case preferences, shortcuts, personalization, data

    var id: String { rawValue }
    var title: String {
        switch self {
        case .preferences: L("Preferences")
        case .shortcuts: L("Shortcuts")
        case .personalization: L("Personalization")
        case .data: L("Data & Help")
        }
    }
    var sections: [SettingsSection] {
        switch self {
        case .preferences: [.general, .search, .appearance, .windowManagement]
        case .shortcuts: [.appShortcuts, .commandShortcuts, .appleShortcuts]
        case .personalization: [.quicklinks, .snippets, .newFile, .scripts, .extensions, .aliases, .profiles]
        case .data: [.configuration, .clipboard, .help]
        }
    }
}

private struct CommandShortcutTarget: Identifiable, Hashable {
    let id: String
    let title: String
    let symbol: String
}

private struct DisplayOption: Identifiable {
    let id: String
    let name: String
}

private struct AppAliasRow: View {
    let app: IndexedApplication
    @ObservedObject var index: ApplicationIndex
    @ObservedObject var aliases: AppAliasStore
    @State private var draft: String
    @State private var error: String?

    init(app: IndexedApplication, index: ApplicationIndex, aliases: AppAliasStore) {
        self.app = app
        self.index = index
        self.aliases = aliases
        _draft = State(initialValue: aliases.alias(for: app) ?? "")
    }

    var body: some View {
        HStack(spacing: 10) {
            Group {
                if let icon = index.icon(for: app) { Image(nsImage: icon).resizable() }
                else { Image(systemName: "app.fill").resizable().foregroundStyle(.secondary) }
            }.frame(width: 24, height: 24).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(app.name)
                if let error { Text(error).font(.caption).foregroundStyle(.red) }
            }
            Spacer()
            TextField(L("Alias"), text: $draft)
                .frame(width: 180)
                .accessibilityLabel("Alias for \(app.name)")
                .onSubmit(save)
                .onChange(of: aliases.alias(for: app) ?? "") { _, value in draft = value }
            Button(L("Save")) { save() }
            if aliases.alias(for: app) != nil {
                Button(role: .destructive) { aliases.remove(for: app); draft = ""; error = nil } label: {
                    Image(systemName: "xmark.circle")
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("Remove alias for \(app.name)")
            }
        }
    }

    private func save() {
        error = aliases.save(alias: draft, for: app)
        if error == nil { draft = aliases.alias(for: app) ?? "" }
    }
}

struct ShortcutRecorder: NSViewRepresentable {
    var record: (UInt32, UInt32, String) -> Void
    var cancel: () -> Void

    func makeNSView(context: Context) -> ShortcutCaptureView {
        let view = ShortcutCaptureView()
        view.record = record
        view.cancel = cancel
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.textField)
        view.setAccessibilityLabel(L("Shortcut recorder"))
        view.setAccessibilityHelp(L("Press Command, Option, or Control with a key. Press Escape to cancel."))
        DispatchQueue.main.async {
            view.previousFirstResponder = view.window?.firstResponder
            view.window?.makeFirstResponder(view)
        }
        return view
    }

    func updateNSView(_ view: ShortcutCaptureView, context: Context) {
        view.record = record
        view.cancel = cancel
    }

    static func dismantleNSView(_ view: ShortcutCaptureView, coordinator: ()) {
        view.restoreFirstResponderIfNeeded()
        view.record = nil
        view.cancel = nil
    }
}

final class ShortcutCaptureView: NSView {
    var record: ((UInt32, UInt32, String) -> Void)?
    var cancel: (() -> Void)?
    weak var previousFirstResponder: NSResponder?
    override var acceptsFirstResponder: Bool { true }
    override var intrinsicContentSize: NSSize { NSSize(width: 1, height: 1) }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard window?.firstResponder === self else { return false }
        keyDown(with: event)
        return true
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == UInt16(kVK_Escape) { cancel?(); return }
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        var modifiers: UInt32 = 0
        var label = ""
        if flags.contains(.control) { modifiers |= UInt32(controlKey); label += "⌃" }
        if flags.contains(.option) { modifiers |= UInt32(optionKey); label += "⌥" }
        if flags.contains(.shift) { modifiers |= UInt32(shiftKey); label += "⇧" }
        if flags.contains(.command) { modifiers |= UInt32(cmdKey); label += "⌘" }
        guard modifiers & UInt32(cmdKey | optionKey | controlKey) != 0 else { return }
        let special: [UInt16: String] = [UInt16(kVK_Space): "Space", UInt16(kVK_Return): "↩", UInt16(kVK_Tab): "⇥", UInt16(kVK_Delete): "⌫", UInt16(kVK_ForwardDelete): "⌦", UInt16(kVK_LeftArrow): "←", UInt16(kVK_RightArrow): "→", UInt16(kVK_UpArrow): "↑", UInt16(kVK_DownArrow): "↓"]
        let key = special[event.keyCode] ?? event.charactersIgnoringModifiers?.uppercased() ?? "Key \(event.keyCode)"
        record?(UInt32(event.keyCode), modifiers, label + " " + key)
    }

    func restoreFirstResponderIfNeeded() {
        guard window?.firstResponder === self else { return }
        if let previousFirstResponder {
            window?.makeFirstResponder(previousFirstResponder)
        } else {
            window?.makeFirstResponder(nil)
        }
    }
}
