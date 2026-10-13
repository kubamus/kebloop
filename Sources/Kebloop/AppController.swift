import AppKit
import SwiftUI
import ServiceManagement
import Combine

@MainActor
final class Preferences: ObservableObject {
    @Published var shortcutID = UserDefaults.standard.string(forKey: "shortcut") ?? "option"
    @Published var customShortcut = UserDefaults.standard.data(forKey: "launcherCustomShortcut").flatMap { try? JSONDecoder().decode(Shortcut.self, from: $0) }
    @Published var shortcutError: String?
    @Published var loginEnabled = SMAppService.mainApp.status == .enabled
    @Published var loginError: String?
    var shortcut: Shortcut { if shortcutID == "custom", let customShortcut { return customShortcut }; return Shortcut.choices.first { $0.id == shortcutID } ?? Shortcut.choices[0] }
    var applyShortcut: (() -> Void)?
    func saveCustomShortcut(keyCode: UInt32, modifiers: UInt32, label: String) {
        customShortcut = Shortcut(id: "custom", label: label, modifiers: modifiers, keyCode: keyCode)
        if let data = try? JSONEncoder().encode(customShortcut) { UserDefaults.standard.set(data, forKey: "launcherCustomShortcut") }
        shortcutID = "custom"; saveShortcut()
    }
    func saveShortcut() {
        UserDefaults.standard.set(shortcutID, forKey: "shortcut")
        applyShortcut?()
    }
    func setLogin(_ enabled: Bool) {
        do {
            if enabled { try SMAppService.mainApp.register() }
            else { try SMAppService.mainApp.unregister() }
            loginEnabled = SMAppService.mainApp.status == .enabled
            loginError = SMAppService.mainApp.status == .requiresApproval ? "Allow Kebloop in System Settings → General → Login Items." : nil
        } catch {
            loginEnabled = SMAppService.mainApp.status == .enabled
            loginError = error.localizedDescription
        }
    }
}

@MainActor
final class AppController: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private let onboardingNeeded = !UserDefaults.standard.bool(forKey: "onboardingDismissed") && UserDefaults.standard.object(forKey: "automaticLoginConfigured") == nil && UserDefaults.standard.object(forKey: "shortcut") == nil && UserDefaults.standard.object(forKey: "clipboardHistoryEnabled") == nil
    private let preferences = Preferences()
    private let index = ApplicationIndex()
    private let hotkey = HotkeyService()
    private let quicklinks = QuicklinkStore()
    private let clipboard = ClipboardHistoryStore()
    private let snippets = SnippetStore()
    private let scripts = ScriptCommandStore()
    private let extensions = ExtensionStore()
    private var automationWindows: [NSWindow] = []
    private var workflowCleanup: [ObjectIdentifier: () -> Void] = [:]
    private lazy var raycastSDK = RaycastSDKStore()
    private lazy var calendarMeetings = CalendarMeetingService()
    private lazy var markdownNotes = MarkdownNoteStore()
    private var notesPanel: NSPanel?
    private var notesHasUnsavedDraft = false
    private let aliases = AppAliasStore()
    private let ai = AIHandoffService()
    private let personalization = PersonalizationStore()
    private let appleShortcuts = AppleShortcutsService()
    private var commandShortcuts: CommandShortcutStore!
    private var configuration: ConfigurationStore!
    private var subscriptions = Set<AnyCancellable>()
    private var helpWindow: NSWindow?
    private var shortcuts: AppShortcutStore!
    private var emojiPicker: EmojiPickerController?
    private var launcher: LauncherPanelController!
    private var statusItem: NSStatusItem!
    private var settings: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        if onboardingNeeded { clipboard.enabled = false }
        SnippetExpansionService.shared.configure(store: snippets)
        shortcuts = AppShortcutStore(index: index, hotkey: hotkey)
        commandShortcuts = CommandShortcutStore(hotkey: hotkey)
        configuration = ConfigurationStore(reload: { [weak self] in self?.reloadConfiguration() }, validateActivation: { [weak self] in
            guard let self else { return }
            if let error = self.preferences.shortcutError ?? self.shortcuts.errors.values.first ?? self.commandShortcuts.errors.values.first { throw ConfigurationError.invalid(error) }
        })
        shortcuts.isEnabled = { [weak self] binding in
            guard let self else { return false }
            return self.personalization.settings.appsEnabled && !self.personalization.settings.hiddenApps.contains(binding.appPath)
        }
        commandShortcuts.isEnabled = { [weak self] id in
            guard let self, self.personalization.settings.commandsEnabled else { return false }
            if id.hasPrefix("link:") { return self.personalization.settings.linksEnabled && self.quicklinks.links.contains { "link:" + $0.id == id } }
            if id.hasPrefix("script:") { return self.scripts.commands.contains { $0.commandID == id && $0.enabled } }
            if id.hasPrefix("extension:") { return self.extensions.manifests.contains { $0.commandID == id && $0.enabled } }
            if id.hasPrefix("snippet:") { return self.snippets.snippets.contains { "snippet:" + $0.id == id } }
            if id.hasPrefix("apple:") { return self.appleShortcuts.selected.contains { "apple:" + $0.id == id } }
            return LauncherCommand(rawValue: id) != nil
        }
        personalization.objectWillChange.receive(on: DispatchQueue.main).sink { [weak self] _ in
            DispatchQueue.main.async { self?.shortcuts.resume(); self?.commandShortcuts.resume(); self?.applyWindowAppearance() }
        }.store(in: &subscriptions)
        quicklinks.$links.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in DispatchQueue.main.async { self?.reconcileCommandBindings() } }.store(in: &subscriptions)
        appleShortcuts.$selected.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in DispatchQueue.main.async { self?.reconcileCommandBindings() } }.store(in: &subscriptions)
        scripts.$commands.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in DispatchQueue.main.async { self?.reconcileCommandBindings() } }.store(in: &subscriptions)
        extensions.$manifests.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in DispatchQueue.main.async { self?.reconcileCommandBindings() } }.store(in: &subscriptions)
        snippets.$snippets.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in DispatchQueue.main.async { self?.reconcileCommandBindings() } }.store(in: &subscriptions)
        configureMenus()
        launcher = LauncherPanelController(index: index, quicklinks: quicklinks, clipboard: clipboard, aliases: aliases, ai: ai, personalization: personalization, appleShortcuts: appleShortcuts, snippets: snippets, scripts: scripts, extensions: extensions)
        launcher.noteStore = { [weak self] in self?.markdownNotes ?? MarkdownNoteStore() }
        launcher.onNote = { [weak self] id in self?.showNotes(noteID: id) }
        launcher.profileName = { [weak self] in self?.configuration.activeProfileName ?? "Personal" }
        launcher.onScript = { [weak self] command, arguments in self?.runScript(command, arguments: arguments) }
        launcher.onExtension = { [weak self] in self?.showExtension($0) }
        launcher.onSettings = { [weak self] in self?.showSettings() }
        commandShortcuts.onCommand = { [weak self] id in self?.launcher.executeCommand(id: id) }
        launcher.onCommand = { [weak self] command in self?.handleCommand(command) }
        launcher.onEditQuicklink = { [weak self] link in
            self?.showSettings(); DispatchQueue.main.async { NotificationCenter.default.post(name: .init("KebloopEditQuicklink"), object: link) }
        }
        launcher.onEmoji = { [weak self] in self?.showEmoji() }
        launcher.onAI = { [weak self] query in self?.sendToAI(query) }
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem.button?.image = NSImage(systemSymbolName: "command.square", accessibilityDescription: "Kebloop")
        let menu = NSMenu()
        menu.addItem(withTitle: L("Open Kebloop"), action: #selector(openLauncher), keyEquivalent: "")
        menu.addItem(withTitle: LauncherCommand.notes.title, action: #selector(openNotes), keyEquivalent: "")
        menu.addItem(withTitle: LauncherCommand.newNote.title, action: #selector(createNote), keyEquivalent: "")
        menu.addItem(withTitle: LauncherCommand.dictionary.title, action: #selector(openDictionary), keyEquivalent: "")
        menu.addItem(withTitle: LauncherCommand.dictation.title, action: #selector(openDictation), keyEquivalent: "")
        menu.addItem(withTitle: L("Settings…"), action: #selector(showSettings), keyEquivalent: ",")
        menu.addItem(withTitle: L("Refresh Applications"), action: #selector(refresh), keyEquivalent: "")
        menu.addItem(.separator())
        menu.addItem(withTitle: L("Quit Kebloop"), action: #selector(quit), keyEquivalent: "")
        for item in menu.items { item.target = self }
        statusItem.menu = menu
        hotkey.onPress = { [weak self] in self?.launcher.toggle() }
        preferences.applyShortcut = { [weak self] in self?.registerShortcut() }
        registerShortcut()
        index.refresh()
        let defaults = UserDefaults.standard
        if !defaults.bool(forKey: "onboardingDismissed") {
            // Existing installations retain login and privacy choices without a new walkthrough.
            if !onboardingNeeded {
                defaults.set(true, forKey: "onboardingDismissed")
            } else { DispatchQueue.main.async { [weak self] in self?.showHelp(onboarding: true) } }
        }
        // A normal manual launch opens the panel. Login launches stay in the menu bar.
        let launchedAtLogin = NSAppleEventManager.shared().currentAppleEvent?.paramDescriptor(forKeyword: keyAEPropData)?.enumCodeValue == type(general: "logi")
        if !launchedAtLogin { launcher.show() }
    }
    private func configureMenus() {
        let bar = NSMenu()
        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        let settingsItem = appMenu.addItem(withTitle: L("Settings…"), action: #selector(showSettings), keyEquivalent: ",")
        settingsItem.target = self
        appMenu.addItem(.separator())
        let hideItem = appMenu.addItem(withTitle: L("Hide Kebloop"), action: #selector(hideInterface), keyEquivalent: "q")
        hideItem.target = self
        let quitItem = appMenu.addItem(withTitle: L("Quit Kebloop"), action: #selector(quit), keyEquivalent: "")
        quitItem.target = self
        appItem.submenu = appMenu
        bar.addItem(appItem)
        let editItem = NSMenuItem()
        let editMenu = NSMenu(title: L("Edit Menu"))
        for (title, selector, key) in [("Undo", "undo:", "z"), ("Redo", "redo:", "Z"),
                                       ("Cut", "cut:", "x"), ("Copy", "copy:", "c"),
                                       ("Paste", "paste:", "v"), ("Select All", "selectAll:", "a")] {
            let item = editMenu.addItem(withTitle: L(title), action: Selector(selector), keyEquivalent: key)
            item.keyEquivalentModifierMask = key == "Z" ? [.command, .shift] : .command
        }
        editItem.submenu = editMenu
        bar.addItem(editItem)
        NSApp.mainMenu = bar
    }
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        if sender === notesPanel, notesHasUnsavedDraft { return false }
        return true
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if notesHasUnsavedDraft {
            notesPanel?.makeKeyAndOrderFront(nil)
            return .terminateCancel
        }
        return .terminateNow
    }

    func windowWillClose(_ notification: Notification) {
        if let window = notification.object as? NSWindow {
            if window === notesPanel { markdownNotes.flushAll() }
            workflowCleanup.removeValue(forKey: ObjectIdentifier(window))?()
            automationWindows.removeAll { $0 === window }
        }
        shortcuts?.resume(); commandShortcuts?.resume()
    }
    private func type(general: String) -> UInt32 { general.utf8.reduce(0) { ($0 << 8) | UInt32($1) } }
    private func registerShortcut() {
        shortcuts?.suspend()
        preferences.shortcutError = hotkey.register(preferences.shortcut) ? nil : "This shortcut is unavailable. Choose another shortcut."
        shortcuts?.resume()
        commandShortcuts?.resume()
    }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        launcher.show()
        return false
    }
    private func sendToAI(_ query: String) {
        launcher.dismiss(restoreFocus: false)
        ai.handoff(query) { [weak self] error in
            if let error { self?.launcher.showHandoffError(query: query, error: error) }
        }
    }
    private func showEmoji() {
        launcher.dismiss(restoreFocus: false)
        if emojiPicker == nil {
            emojiPicker = EmojiPickerController(onCopy: { [weak self] in self?.launcher.restorePreviousFocus() }, onPaste: { [weak self] in self?.launcher.pasteIntoPreviousApplication() })
        }
        emojiPicker?.show()
    }
    private func reconcileCommandBindings() {
        let validLinks = Set(quicklinks.links.map { "link:" + $0.id })
        let validShortcuts = Set(appleShortcuts.selected.map { "apple:" + $0.id })
        for binding in commandShortcuts.bindings {
            if binding.id.hasPrefix("script:"), !scripts.commands.contains(where: { $0.commandID == binding.id }) { commandShortcuts.remove(id: binding.id) }
            if binding.id.hasPrefix("extension:"), !extensions.manifests.contains(where: { $0.commandID == binding.id }) { commandShortcuts.remove(id: binding.id) }
            if binding.id.hasPrefix("snippet:"), !snippets.snippets.contains(where: { "snippet:" + $0.id == binding.id }) { commandShortcuts.remove(id: binding.id) }
            if binding.id.hasPrefix("link:"), !validLinks.contains(binding.id) { commandShortcuts.remove(id: binding.id) }
            if binding.id.hasPrefix("apple:"), !validShortcuts.contains(binding.id) { commandShortcuts.remove(id: binding.id) }
        }
        commandShortcuts.resume()
    }
    private func reloadConfiguration() {
        shortcuts.suspend()
        commandShortcuts.suspend()
        shortcuts.unregisterAll(); commandShortcuts.unregisterAll()
        preferences.customShortcut = UserDefaults.standard.data(forKey: "launcherCustomShortcut").flatMap { try? JSONDecoder().decode(Shortcut.self, from: $0) }
        preferences.shortcutID = UserDefaults.standard.string(forKey: "shortcut") ?? "option"
        preferences.shortcutError = hotkey.register(preferences.shortcut) ? nil : "Launcher shortcut unavailable."
        scripts.reload(); extensions.reload(); snippets.reload(); aliases.reload(); quicklinks.reload(); personalization.reload(); appleShortcuts.reload()
        shortcuts.reload(register: false); commandShortcuts.reload(register: false)
        shortcuts.resume(); commandShortcuts.resume()
    }
    private func automationWindow<V: View>(title: String, view: V, onClose: (() -> Void)? = nil) {
        launcher.dismiss(restoreFocus: false)
        automationWindows.removeAll { !$0.isVisible }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 800, height: 640), styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = title; window.isReleasedWhenClosed = false
        window.contentMinSize = NSSize(width: 740, height: 540)
        window.delegate = self
        workflowCleanup[ObjectIdentifier(window)] = onClose
        window.contentView = NSHostingView(rootView: view)
        automationWindows.append(window); window.center(); window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
    }
    private func runScript(_ command: ScriptCommand, arguments: [String] = []) {
        automationWindow(title: command.title, view: ScriptOutputView(store: scripts))
        scripts.run(id: command.id, arguments: arguments)
    }
    private func showExtension(_ manifest: ExtensionManifest) {
        automationWindow(title: manifest.name, view: ExtensionBrowserView(store: extensions, extensionID: manifest.id))
    }
    private func handleCommand(_ command: LauncherCommand) {
        switch command {
        case .calendar:
            automationWindow(title: command.title, view: CalendarMeetingSettingsView(service: calendarMeetings))
        case .joinNextMeeting:
            launcher.dismiss(restoreFocus: false)
            Task { @MainActor [weak self] in
                guard let self else { return }
                if let message = await self.calendarMeetings.joinNextMeeting() {
                    self.automationWindow(title: command.title, view: VStack(alignment: .leading) {
                        Text(message).padding()
                        CalendarMeetingSettingsView(service: self.calendarMeetings)
                    })
                }
            }
        case .selectedText:
            let target = launcher.previousApplication
            let service = SelectedTextActionsService()
            automationWindow(title: command.title, view: SelectedTextActionsView(application: target, service: service), onClose: { service.cancelTransform() })
        case .newNote:
            guard !notesHasUnsavedDraft else { notesPanel?.makeKeyAndOrderFront(nil); return }
            if let note = markdownNotes.create(contents: "") { showNotes(noteID: note.id, editing: true) }
            else { showNotes() }
        case .notes, .dictionary, .dictation: launcher.executeCommand(id: command.id)
        case .raycastImport:
            automationWindow(title: command.title, view: RaycastMigrationView(snippets: snippets, quicklinks: quicklinks))
        case .raycastExtensions:
            automationWindow(title: command.title, view: RaycastCompatibleCommandsView(scripts: scripts, sdk: raycastSDK), onClose: { [weak self] in self?.raycastSDK.cancel() })
        case .windowLayouts:
            launcher.dismiss(restoreFocus: false)
            SavedWindowLayoutsWindowController.shared.show()
        case .windowSearch, .menuSearch:
            let target = launcher.previousApplication
            launcher.dismiss(restoreFocus: false)
            WindowMenuSearchWindowController.shared.show(application: target, scope: command == .windowSearch ? .windows : .menus)
        case .browse, .help: showHelp(onboarding: false)
        case .onboarding: showHelp(onboarding: true)
        case .snippets, .scripts, .extensions, .profiles, .appleShortcuts, .webSearch:
            showSettings(); DispatchQueue.main.async { NotificationCenter.default.post(name: .init("KebloopSettingsTab"), object: command.rawValue) }
        default: launcher.executeCommand(id: command.rawValue)
        }
    }
    private func showNotes(noteID: String? = nil, editing: Bool = false) {
        guard !notesHasUnsavedDraft else { notesPanel?.makeKeyAndOrderFront(nil); return }
        launcher.dismiss(restoreFocus: false)
        if notesPanel == nil {
            let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 620, height: 440),
                                styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
            panel.contentMinSize = NSSize(width: 480, height: 320)
            panel.title = LauncherCommand.notes.title
            panel.isReleasedWhenClosed = false
            panel.level = .floating
            panel.hidesOnDeactivate = false
            panel.delegate = self
            panel.center()
            notesPanel = panel
        }
        markdownNotes.flushAll()
        notesPanel?.contentView = NSHostingView(rootView: MarkdownNotesView(store: markdownNotes,
            initialNoteID: noteID, initiallyEditing: editing, onClose: { [weak self] in
                self?.notesPanel?.close(); self?.launcher.restorePreviousFocus()
            }, onUnsavedDraftChanged: { [weak self] pending in self?.notesHasUnsavedDraft = pending },
            onBrowse: { [weak self] in
                self?.notesPanel?.close()
                self?.launcher.executeCommand(id: LauncherCommand.notes.id)
            }))
        notesPanel?.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    private func showHelp(onboarding: Bool) {
        launcher.dismiss(restoreFocus: false)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 520), styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = onboarding ? "Welcome to Kebloop" : "Kebloop Commands and Help"
        window.isReleasedWhenClosed = false
        if onboarding {
            window.contentView = NSHostingView(rootView: OnboardingView(preferences: preferences, clipboard: clipboard, onFinish: { [weak self, weak window] in
                UserDefaults.standard.set(true, forKey: "onboardingDismissed"); window?.close(); self?.launcher.show()
            }))
        } else {
            window.contentView = NSHostingView(rootView: HelpView(personalization: personalization, quicklinks: quicklinks, commandShortcuts: commandShortcuts, appleShortcuts: appleShortcuts, appShortcuts: shortcuts, clipboard: clipboard, launcherShortcutLabel: preferences.shortcut.label, onReplayOnboarding: { [weak self, weak window] in
                window?.close(); self?.showHelp(onboarding: true)
            }, onExecute: { [weak self, weak window] id in
                window?.close(); self?.launcher.executeCommand(id: id)
            }))
        }
        helpWindow = window
        window.center(); NSApp.activate(ignoringOtherApps: true); window.makeKeyAndOrderFront(nil)
    }

    @objc private func openNotes() { launcher.executeCommand(id: LauncherCommand.notes.id) }
    @objc private func createNote() { launcher.executeCommand(id: LauncherCommand.newNote.id) }
    @objc private func openDictionary() { launcher.executeCommand(id: LauncherCommand.dictionary.id) }
    @objc private func openDictation() { launcher.executeCommand(id: LauncherCommand.dictation.id) }
    @objc private func openLauncher() { launcher.show() }
    @objc private func refresh() { index.refresh() }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    @objc private func hideInterface() {
        emojiPicker?.dismiss()
        settings?.orderOut(nil)
        launcher.dismiss()
    }
    func applicationWillTerminate(_ notification: Notification) { markdownNotes.flushAll(); for cleanup in workflowCleanup.values { cleanup() }; scripts.cancel(); extensions.cancel() }
    @objc private func quit() { scripts.cancel(); extensions.cancel(); NSApp.terminate(nil) }
    private func applyWindowAppearance() {
        let appearance: NSAppearance?
        switch personalization.settings.appearance {
        case "light": appearance = NSAppearance(named: .aqua)
        case "dark": appearance = NSAppearance(named: .darkAqua)
        default: appearance = nil
        }
        settings?.appearance = appearance
        helpWindow?.appearance = appearance
        notesPanel?.appearance = appearance
        for window in automationWindows { window.appearance = appearance }
    }
    @objc func showSettings() {
        launcher.dismiss(restoreFocus: false)
        if settings == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 940, height: 680), styleMask: [.titled, .closable, .resizable, .miniaturizable], backing: .buffered, defer: false)
            window.title = L("Kebloop Settings")
            window.contentMinSize = NSSize(width: 820, height: 580)
            window.isReleasedWhenClosed = false
            window.delegate = self
            window.contentView = NSHostingView(rootView: SettingsView(preferences: preferences, index: index, quicklinks: quicklinks, shortcuts: shortcuts, aliases: aliases, clipboard: clipboard, ai: ai, personalization: personalization, configuration: configuration, commandShortcuts: commandShortcuts, appleShortcuts: appleShortcuts, snippets: snippets, scripts: scripts, extensions: extensions, onReplayOnboarding: { [weak self] in self?.showHelp(onboarding: true) }, onExecute: { [weak self] id in
                self?.settings?.orderOut(nil); self?.launcher.executeCommand(id: id)
            }))
            window.center()
            settings = window
        }
        applyWindowAppearance()
        NSApp.activate(ignoringOtherApps: true)
        settings?.makeKeyAndOrderFront(nil)
    }
}
