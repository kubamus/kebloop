import AppKit
import Combine
import SwiftUI
import KebloopCore
import os
import UniformTypeIdentifiers

private final class LauncherPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

// Menu-bar apps have no Edit menu to dispatch the usual field-editor shortcuts.
private final class SearchField: NSTextField {
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection([.command, .option, .control, .shift])
        guard let editor = currentEditor() as? NSTextView else {
            return super.performKeyEquivalent(with: event)
        }
        if modifiers == .command {
            switch event.charactersIgnoringModifiers?.lowercased() {
            case "a": editor.selectAll(nil)
            case "c": editor.copy(nil)
            case "x": editor.cut(nil)
            case "v": editor.paste(nil)
            case "z": editor.undoManager?.undo()
            default: return super.performKeyEquivalent(with: event)
            }
            return true
        }
        if modifiers == [.command, .shift], event.charactersIgnoringModifiers?.lowercased() == "z" {
            editor.undoManager?.redo()
            return true
        }
        return super.performKeyEquivalent(with: event)
    }
}

private enum LauncherMode: String {
    case root, clipboard, quicklinks, webSearch, snippets, calculationHistory, notes, dictionary, dictation, localAssistant, chatHistory
    var title: String {
        switch self {
        case .chatHistory: return L("Chat History")
        case .localAssistant: return L("Local Assistant")
        case .notes: return L("Notes")
        case .dictionary: return L("Dictionary")
        case .dictation: return L("Dictation")
        case .calculationHistory: return L("Calculator History")
        case .snippets: return L("Snippets")
        case .root: return L("Results")
        case .clipboard: return L("Clipboard History")
        case .quicklinks: return L("Quicklinks")
        case .webSearch: return L("Web Search")
        }
    }
    var placeholder: String {
        switch self {
        case .chatHistory: return L("Search conversations…")
        case .localAssistant: return L("Ask a follow-up…  Return to send")
        case .notes: return L("Search notes…  ⌘N New Note")
        case .dictionary: return L("Type a word, then press Return…")
        case .dictation: return L("Dictation · ⌘Return Start or stop")
        case .calculationHistory: return L("Search calculation history…")
        case .snippets: return L("Search snippets…")
        case .root: return L("Search apps, links, or commands…")
        case .clipboard: return L("Search clipboard history…")
        case .quicklinks: return L("Search quicklinks…")
        case .webSearch: return L("Search the web…")
        }
    }
}

private enum LauncherResult {
    case application(IndexedApplication)
    case calculation(CalculationResult)
    case historicalCalculation(CalculationHistoryEntry)
    case command(LauncherCommand)
    case quicklink(Quicklink)
    case clipboard(ClipboardEntry)
    case note(MarkdownNote)
    case snippet(Snippet)
    case script(ScriptCommand)
    case userExtension(ExtensionManifest)
    case file(FileSearchResult)
    case appleShortcut(AppleShortcut)
    case webSearch(String)
    var id: String {
        switch self {
        case .application(let app): return app.id
        case .calculation: return "calculator"
        case .historicalCalculation(let entry): return "calculation-history:" + entry.id
        case .command(let command): return "command:\(command.rawValue)"
        case .quicklink(let link): return "link:\(link.id)"
        case .script(let script): return script.commandID
        case .userExtension(let manifest): return manifest.commandID
        case .note(let note): return "note:" + note.id
        case .snippet(let snippet): return "snippet:\(snippet.id)"
        case .clipboard(let entry): return "clipboard:\(entry.id)"
        case .file(let file): return "file:\(file.id)"
        case .appleShortcut(let shortcut): return "apple:\(shortcut.id)"
        case .webSearch: return "web-search"
        }
    }
}

private final class FlippedStackView: NSStackView {
    override var isFlipped: Bool { true }
}

private final class ResultButton: NSButton {
    var invoke: (() -> Void)?
    var contextInvoke: (() -> Void)?
    override func rightMouseDown(with event: NSEvent) { contextInvoke?() }
    var showsHint = true
    weak var selectionHint: NSTextField?

    func setSelected(_ selected: Bool) {
        layer?.backgroundColor = selected ? NSColor.controlAccentColor.withAlphaComponent(NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast ? 0.35 : 0.14).cgColor : NSColor.clear.cgColor
        layer?.borderWidth = selected && NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast ? 2 : 0
        layer?.borderColor = NSColor.controlAccentColor.cgColor
        selectionHint?.stringValue = selected && showsHint ? "↵" : ""
        setAccessibilityValue(selected ? "Selected" : "")
    }
    override var acceptsFirstResponder: Bool { true }
    init() {
        super.init(frame: .zero)
        isBordered = false
        title = ""
        target = self
        action = #selector(clicked)
        setAccessibilityRole(.button)
        focusRingType = .exterior
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func clicked() { invoke?() }
}

@MainActor
final class LauncherPanelController: NSObject, NSTextFieldDelegate, NSWindowDelegate {
    var noteStore: (() -> MarkdownNoteStore)?
    var onScript: ((ScriptCommand, [String]) -> Void)?
    var onExtension: ((ExtensionManifest) -> Void)?
    var onSettings: (() -> Void)?
    var onEmoji: (() -> Void)?
    var onAI: ((String) -> Void)?
    var profileName: (() -> String)?
    var onCommand: ((LauncherCommand) -> Void)?
    var onEditQuicklink: ((Quicklink) -> Void)?
    private let personalization: PersonalizationStore
    private let appleShortcuts: AppleShortcutsService
    private let actionsMenu = ActionMenuController()
    private var actionMenuOpen: Bool { actionsMenu.isVisible }
    private let actionButton = NSButton(title: L("Actions") + " ⌘K", target: nil, action: nil)
    private var recentIDs = UserDefaults.standard.stringArray(forKey: "recentResults") ?? []
    private let launchApplication: (IndexedApplication, @escaping (Error?) -> Void) -> Void
    private var presentationRevision = 0
    private struct ExternalActionContext {
        let query: String
        let mode: LauncherMode
        let resultID: String
        let revision: Int
    }
    private let index: ApplicationIndex
    private let quicklinks: QuicklinkStore
    private let clipboard: ClipboardHistoryStore
    private let snippets: SnippetStore
    private let scripts: ScriptCommandStore
    private let extensions: ExtensionStore
    private let aliases: AppAliasStore
    private let ai: AIHandoffService
    private let assistantTools = LocalAssistantTools()
    private let assistantOverride: LocalAssistantService?
    private(set) lazy var localAssistant = assistantOverride ?? LocalAssistantService(tools: assistantTools)
    private let aiButton = NSButton(title: "", target: nil, action: nil)
    private let clearHistoryButton = NSButton(title: L("Clear…"), target: nil, action: nil)
    private var isConfirmingClear = false
    private let files = FileSearchService()
    private var mode: LauncherMode = .root
    private static let performanceLog = OSLog(subsystem: "com.kebloop.launcher", category: .pointsOfInterest)
    private let search = AppSearch()
    private let calculator = CalculatorEngine()
    private let currencyRates: CurrencyRateStore
    private let calculationHistory: CalculationHistoryStore
    private let pasteService = PasteService()
    private let windowManagement = WindowManagementService()
    private let panel: LauncherPanel
    private let input = SearchField()
    private let utilityHost = NSView()
    private var utilityHostingView: NSView?
    private var utilityHeight: NSLayoutConstraint!
    private var loadedNotes: MarkdownNoteStore?
    private var notesSubscription: AnyCancellable?
    private var inlineDictation: OnDeviceDictationService?
    private var dictionaryDefinition: String?
    private var dictionaryLoading = false
    private var dictionaryLookedUp = false
    private var dictionaryRevision = UUID()
    private var dictionaryTask: Task<Void, Never>?
    private var dictationDeliveryTask: Task<Void, Never>?
    private let lookupDefinition: @Sendable (String) -> String?
    private let rows = FlippedStackView()
    private let scrollView = NSScrollView()
    private let resultsBody = NSStackView()
    private let notePreviewHost = NSView()
    private var notePreviewHostingView: NSView?
    private var noteEditorSession: MarkdownNoteSession?
    private var notesHasUnsavedDraft = false
    var currentPreviewNoteID: String? {
        guard mode == .notes else { return nil }
        if let session = noteEditorSession { return session.selectedNoteID }
        guard results.indices.contains(selection), case .note(let note) = results[selection] else { return nil }
        return note.id
    }
    var currentEditingNoteID: String? { noteEditorSession?.editingNoteID }
    var hasUnsavedNoteDraft: Bool { notesHasUnsavedDraft }
    func saveNotes() -> Bool { noteEditorSession?.saveAndPreview() ?? true }
    private var scrollHeight: NSLayoutConstraint!
    private let backButton = NSButton()
    private let searchIcon = NSImageView()
    private let category = NSTextField(labelWithString: "APPLICATIONS")
    private var results: [LauncherResult] = []
    var currentModeID: String { mode.rawValue }
    var currentDictionaryDefinition: String? { dictionaryDefinition }
    var currentQuery: String { input.stringValue }
    var resultIdentifiers: [String] { results.map(\.id) }
    private var selection = 0
    private var usage = UserDefaults.standard.dictionary(forKey: "usage") as? [String: Int] ?? [:]
    private var activeScreen: NSScreen?
    private var previousApp: NSRunningApplication?
    private var subscriptions = Set<AnyCancellable>()
    private var keyboardMonitor: Any?
    private var outsideMonitor: Any?
    private var message: String?

    init(index: ApplicationIndex, quicklinks: QuicklinkStore, clipboard: ClipboardHistoryStore, aliases: AppAliasStore, ai: AIHandoffService, personalization: PersonalizationStore? = nil, appleShortcuts: AppleShortcutsService? = nil, snippets: SnippetStore? = nil, scripts: ScriptCommandStore? = nil, extensions: ExtensionStore? = nil, currencyRates: CurrencyRateStore? = nil, calculationHistory: CalculationHistoryStore? = nil, launchApplication: ((IndexedApplication, @escaping (Error?) -> Void) -> Void)? = nil, dictationService: OnDeviceDictationService? = nil, localAssistant: LocalAssistantService? = nil, lookupDefinition: @escaping @Sendable (String) -> String? = { DictionaryLookupService.definition(for: $0) }) {
        self.index = index
        self.assistantOverride = localAssistant
        self.inlineDictation = dictationService
        self.lookupDefinition = lookupDefinition
        self.currencyRates = currencyRates ?? CurrencyRateStore()
        self.calculationHistory = calculationHistory ?? CalculationHistoryStore()
        self.launchApplication = launchApplication ?? index.launch
        self.quicklinks = quicklinks
        self.clipboard = clipboard
        self.snippets = snippets ?? SnippetStore()
        self.scripts = scripts ?? ScriptCommandStore()
        self.extensions = extensions ?? ExtensionStore()
        self.aliases = aliases
        self.ai = ai
        self.personalization = personalization ?? PersonalizationStore()
        self.appleShortcuts = appleShortcuts ?? AppleShortcutsService()
        panel = LauncherPanel(contentRect: NSRect(x: 0, y: 0, width: 560, height: 360), styleMask: [.borderless], backing: .buffered, defer: false)
        super.init()
        assistantTools.catalogHandler = { [weak self] query in
            guard let self else { throw AutomationFailure.cancelled }
            return self.assistantCatalog(query: query)
        }
        assistantTools.executeHandler = { [weak self] id, input in
            guard let self else { throw AutomationFailure.cancelled }
            return try await self.executeAssistantAction(id: id, input: input)
        }
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.delegate = self
        NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)
            .sink { [weak self] _ in
                guard let self, self.panel.isVisible, !self.actionsMenu.isVisible else { return }
                self.focusInput()
            }.store(in: &subscriptions)
        panel.setAccessibilityLabel(L("Kebloop launcher"))
        buildInterface()
        self.personalization.objectWillChange.receive(on: DispatchQueue.main).sink { [weak self] _ in
            DispatchQueue.main.async { self?.applyPersonalization() }
        }.store(in: &subscriptions)
        for publisher in [self.scripts.objectWillChange, self.extensions.objectWillChange] {
            publisher.receive(on: DispatchQueue.main).sink { [weak self] _ in
                DispatchQueue.main.async { guard let self, self.panel.isVisible else { return }; self.updateResults(preserveSelection: true) }
            }.store(in: &subscriptions)
        }
        self.snippets.objectWillChange.receive(on: DispatchQueue.main).sink { [weak self] _ in
            DispatchQueue.main.async { guard let self, self.panel.isVisible else { return }; self.updateResults(preserveSelection: true) }
        }.store(in: &subscriptions)
        self.appleShortcuts.objectWillChange.receive(on: DispatchQueue.main).sink { [weak self] _ in
            DispatchQueue.main.async { guard let self, self.panel.isVisible else { return }; self.updateResults(preserveSelection: true) }
        }.store(in: &subscriptions)
        NotificationCenter.default.publisher(for: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification).sink { [weak self] _ in self?.applyPersonalization() }.store(in: &subscriptions)
        for name in [NSWorkspace.didLaunchApplicationNotification, NSWorkspace.didTerminateApplicationNotification, NSWorkspace.didHideApplicationNotification, NSWorkspace.didUnhideApplicationNotification] {
            NSWorkspace.shared.notificationCenter.publisher(for: name).receive(on: DispatchQueue.main).sink { [weak self] _ in
                guard let self, self.panel.isVisible else { return }; self.render()
            }.store(in: &subscriptions)
        }
        index.$applications.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in guard let self, self.panel.isVisible else { return }; self.updateResults(preserveSelection: true) }.store(in: &subscriptions)
        index.$iconRevision.dropFirst().throttle(for: .milliseconds(30), scheduler: DispatchQueue.main, latest: true).sink { [weak self] _ in
            guard let self, self.panel.isVisible else { return }; self.render()
        }.store(in: &subscriptions)
        quicklinks.$links.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in guard let self, self.panel.isVisible else { return }; self.updateResults(preserveSelection: true) }.store(in: &subscriptions)
        self.currencyRates.$snapshots.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in
            guard let self, self.panel.isVisible, self.mode == .root else { return }
            self.message = nil; self.updateResults(preserveSelection: true)
        }.store(in: &subscriptions)
        self.calculationHistory.$entries.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in
            guard let self, self.panel.isVisible, self.mode == .calculationHistory else { return }
            self.updateResults(preserveSelection: true)
        }.store(in: &subscriptions)
        clipboard.$items.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in
            guard let self, self.panel.isVisible, self.mode == .clipboard else { return }
            self.updateResults(preserveSelection: true)
        }.store(in: &subscriptions)
        files.$results.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in
            guard let self, self.panel.isVisible, self.mode == .root else { return }
            self.updateResults(preserveSelection: true)
        }.store(in: &subscriptions)
        files.$isSearching.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in
            guard let self, self.panel.isVisible, self.mode == .root, self.results.isEmpty else { return }; self.render()
        }.store(in: &subscriptions)
        aliases.$aliases.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in
            guard let self, self.panel.isVisible else { return }; self.updateResults(preserveSelection: false)
        }.store(in: &subscriptions)
        ai.$provider.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in guard let self, self.panel.isVisible else { return }; self.render() }.store(in: &subscriptions)
        keyboardMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self, self.panel.isKeyWindow || self.actionsMenu.isKeyWindow else { return event }
            if self.actionsMenu.isVisible {
                if self.actionsMenu.handleKey(event) { return nil }
                return event
            }
            if (self.panel.firstResponder as? NSTextView)?.hasMarkedText() == true { return event }
            if self.handleNoteDeleteKey(event) { return nil }
            if event.modifierFlags.intersection([.command, .option, .control]) == .command,
               event.charactersIgnoringModifiers?.lowercased() == "n" {
                if event.modifierFlags.contains(.shift) { self.enter(.notes) }
                else { self.createNote() }
                return nil
            }
            if self.mode == .notes, self.noteEditorSession != nil {
                if LauncherKeyboard.isActionShortcut(event) { self.showActions(); return nil }
                return event
            }
            if self.mode == .dictionary || self.mode == .dictation || self.mode == .localAssistant || self.mode == .chatHistory {
                return self.handleUtilityKey(event) ? nil : event
            }
            if LauncherKeyboard.isActionShortcut(event) { self.showActions(); return nil }
            if event.modifierFlags.contains(.command), event.charactersIgnoringModifiers == "," {
                self.onSettings?(); return nil
            }
            if event.modifierFlags.contains(.command), event.charactersIgnoringModifiers?.lowercased() == "q" {
                self.dismiss(); return nil
            }
            if self.mode == .clipboard, event.modifierFlags.contains(.command), event.keyCode == 51 {
                if event.modifierFlags.contains(.shift) { self.confirmClearHistory() }
                else { self.removeSelectedClipboardItem() }
                return nil
            }
            if event.keyCode == 48, NSApp.isFullKeyboardAccessEnabled { return event }
            if event.keyCode == 48,
               event.modifierFlags.intersection([.command, .control, .option, .shift]).isEmpty,
               self.mode == .root,
               (self.input.currentEditor() as? NSTextView)?.hasMarkedText() != true,
               (self.ai.provider == .local || !self.input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) {
                self.askAI()
                return nil
            }
            switch event.keyCode {
            case 53:
                if self.mode == .root { self.dismiss() } else { self.enter(.root) }
                return nil
            case 125: self.moveSelection(1); return nil
            case 126: self.moveSelection(-1); return nil
            case 36, 76:
                if self.mode == .clipboard, event.modifierFlags.contains(.command), self.results.indices.contains(self.selection) {
                    let action: ResultAction = event.modifierFlags.contains(.shift) ? .pastePlainText : .copy
                    self.performResultAction(action, resultID: self.results[self.selection].id)
                    return nil
                }
                if let button = self.panel.firstResponder as? ResultButton { button.invoke?() }
                else if let button = self.panel.firstResponder as? NSButton { button.performClick(nil) }
                else { self.performSelected() }
                return nil
            default: return event
            }
        }
        outsideMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            guard let self else { return }
            if self.mode == .dictation, self.inlineDictation?.state == .requestingPermission { return }
            self.dismiss(restoreFocus: false)
        }
    }

    deinit {
        if let keyboardMonitor { NSEvent.removeMonitor(keyboardMonitor) }
        if let outsideMonitor { NSEvent.removeMonitor(outsideMonitor) }
    }

    private func buildInterface() {
        scrollHeight?.isActive = false
        let content = NSView()
        if !NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency, personalization.settings.transparency, #available(macOS 26.0, *) {
            let glass = NSGlassEffectView()
            glass.style = .regular
            glass.cornerRadius = 20
            // Clip the window backing as well as the glass effect's rounded shape.
            glass.wantsLayer = true
            glass.layer?.cornerRadius = glass.cornerRadius
            glass.layer?.masksToBounds = true
            if #available(macOS 27.0, *) { glass.effectIsInteractive = true }
            glass.contentView = content
            panel.contentView = glass
        } else {
            let material = NSVisualEffectView()
            material.material = .popover
            material.blendingMode = .behindWindow
            material.state = NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency || !personalization.settings.transparency ? .inactive : .active
            material.wantsLayer = true
            material.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
            material.wantsLayer = true
            material.layer?.cornerRadius = 20
            material.layer?.masksToBounds = true
            panel.contentView = material
            content.translatesAutoresizingMaskIntoConstraints = false
            material.addSubview(content)
            NSLayoutConstraint.activate([
                content.leadingAnchor.constraint(equalTo: material.leadingAnchor),
                content.trailingAnchor.constraint(equalTo: material.trailingAnchor),
                content.topAnchor.constraint(equalTo: material.topAnchor),
                content.bottomAnchor.constraint(equalTo: material.bottomAnchor)
            ])
        }

        let container = NSStackView()
        container.orientation = .vertical
        container.alignment = .leading
        container.spacing = 0
        container.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(container)
        NSLayoutConstraint.activate([
            container.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            container.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            container.topAnchor.constraint(equalTo: content.topAnchor),
            container.bottomAnchor.constraint(equalTo: content.bottomAnchor)
        ])

        let header = NSView()
        let icon = searchIcon
        icon.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)
        backButton.image = NSImage(systemSymbolName: "chevron.left", accessibilityDescription: "Back")
        backButton.isBordered = false
        backButton.target = self
        backButton.action = #selector(goBack)
        backButton.isHidden = true
        backButton.setAccessibilityLabel(L("Back to launcher"))
        icon.contentTintColor = .secondaryLabelColor
        input.placeholderString = mode.placeholder
        input.font = .systemFont(ofSize: 19, weight: .regular)
        input.isBordered = false
        input.drawsBackground = false
        input.focusRingType = .none
        input.delegate = self
        input.setAccessibilityLabel(L("Search"))
        input.cell?.isScrollable = true
        for view in [icon, backButton, input] { view.translatesAutoresizingMaskIntoConstraints = false; header.addSubview(view) }
        NSLayoutConstraint.activate([
            header.heightAnchor.constraint(equalToConstant: 56),
            backButton.leadingAnchor.constraint(equalTo: icon.leadingAnchor),
            backButton.centerYAnchor.constraint(equalTo: icon.centerYAnchor),
            backButton.widthAnchor.constraint(equalToConstant: 20),
            backButton.heightAnchor.constraint(equalToConstant: 24),
            icon.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 16),
            icon.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 16), icon.heightAnchor.constraint(equalToConstant: 16),
            input.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 10),
            input.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -16),
            input.centerYAnchor.constraint(equalTo: header.centerYAnchor)
        ])
        container.addArrangedSubview(header)
        addDivider(to: container)
        category.font = .systemFont(ofSize: 10, weight: .semibold)
        category.textColor = .tertiaryLabelColor
        let categoryWrapper = NSView()
        category.translatesAutoresizingMaskIntoConstraints = false
        categoryWrapper.addSubview(category)
        clearHistoryButton.target = self
        clearHistoryButton.action = #selector(confirmClearHistory)
        clearHistoryButton.isBordered = false
        clearHistoryButton.font = .systemFont(ofSize: 11)
        clearHistoryButton.translatesAutoresizingMaskIntoConstraints = false
        clearHistoryButton.setAccessibilityLabel(L("Clear clipboard history"))
        categoryWrapper.addSubview(clearHistoryButton)
        aiButton.target = self
        aiButton.action = #selector(askAI)
        aiButton.isBordered = false
        aiButton.font = .systemFont(ofSize: 11)
        aiButton.translatesAutoresizingMaskIntoConstraints = false
        categoryWrapper.addSubview(aiButton)
        actionButton.target = self; actionButton.action = #selector(showActions)
        actionButton.isBordered = false; actionButton.font = .systemFont(ofSize: 11)
        actionButton.translatesAutoresizingMaskIntoConstraints = false
        categoryWrapper.addSubview(actionButton)
        NSLayoutConstraint.activate([
            categoryWrapper.heightAnchor.constraint(equalToConstant: 22),
            actionButton.trailingAnchor.constraint(equalTo: categoryWrapper.trailingAnchor, constant: -12),
            actionButton.centerYAnchor.constraint(equalTo: categoryWrapper.centerYAnchor),
            aiButton.trailingAnchor.constraint(equalTo: actionButton.leadingAnchor, constant: -12),
            aiButton.centerYAnchor.constraint(equalTo: categoryWrapper.centerYAnchor),
            category.trailingAnchor.constraint(lessThanOrEqualTo: aiButton.leadingAnchor, constant: -8),
            clearHistoryButton.trailingAnchor.constraint(equalTo: actionButton.leadingAnchor, constant: -12),
            clearHistoryButton.centerYAnchor.constraint(equalTo: categoryWrapper.centerYAnchor),
            category.leadingAnchor.constraint(equalTo: categoryWrapper.leadingAnchor, constant: 16),
            category.centerYAnchor.constraint(equalTo: categoryWrapper.centerYAnchor)
        ])
        container.addArrangedSubview(categoryWrapper)
        rows.orientation = .vertical
        rows.alignment = .leading
        rows.spacing = 2
        rows.edgeInsets = NSEdgeInsets(top: 0, left: 6, bottom: 8, right: 6)
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.scrollerStyle = .overlay
        scrollView.autohidesScrollers = true
        scrollView.documentView = rows
        scrollHeight = scrollView.heightAnchor.constraint(equalToConstant: 330)
        scrollHeight.isActive = true
        resultsBody.orientation = .horizontal
        resultsBody.alignment = .top
        resultsBody.spacing = 0
        resultsBody.addArrangedSubview(scrollView)
        resultsBody.addArrangedSubview(notePreviewHost)
        notePreviewHost.isHidden = true
        notePreviewHost.widthAnchor.constraint(equalTo: resultsBody.widthAnchor, multiplier: 0.55).isActive = true
        notePreviewHost.heightAnchor.constraint(equalTo: scrollView.heightAnchor).isActive = true
        container.addArrangedSubview(resultsBody)
        scrollView.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.publisher(for: NSScrollView.didLiveScrollNotification, object: scrollView)
            .sink { [weak self] _ in self?.previewVisibleNote() }.store(in: &subscriptions)
        utilityHeight?.isActive = false
        utilityHeight = utilityHost.heightAnchor.constraint(equalToConstant: 330)
        utilityHeight.isActive = true
        utilityHost.isHidden = true
        container.addArrangedSubview(utilityHost)
        for view in container.arrangedSubviews {
            view.widthAnchor.constraint(equalTo: container.widthAnchor).isActive = true
        }
    }

    private func addDivider(to stack: NSStackView) {
        let line = NSBox()
        line.boxType = .separator
        line.heightAnchor.constraint(equalToConstant: 1).isActive = true
        stack.addArrangedSubview(line)
    }

    func toggle() {
        if panel.isVisible && panel.isKeyWindow && NSApp.isActive { dismiss() }
        else { show() }
    }
    func show() {
        guard saveNotes() else { focusInput(); return }
        noteEditorSession = nil
        inlineDictation?.cancel()
        dictationDeliveryTask?.cancel()
        cancelDictionaryLookup()
        pasteService.cancel()
        presentationRevision &+= 1
        if let frontmost = NSWorkspace.shared.frontmostApplication,
           frontmost.processIdentifier != ProcessInfo.processInfo.processIdentifier {
            previousApp = frontmost
            if let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]],
               let info = windows.first(where: { ($0[kCGWindowOwnerPID as String] as? Int32) == frontmost.processIdentifier && ($0[kCGWindowLayer as String] as? Int) == 0 }),
               let bounds = info[kCGWindowBounds as String] as? [String: CGFloat],
               let x = bounds["X"], let y = bounds["Y"], let width = bounds["Width"], let height = bounds["Height"], let primary = NSScreen.screens.first {
                let center = NSPoint(x: x + width / 2, y: primary.frame.maxY - y - height / 2)
                activeScreen = NSScreen.screens.first { NSMouseInRect(center, $0.frame, false) }
            }

        }
        panel.appearance = personalization.settings.appearance == "light" ? NSAppearance(named: .aqua) : (personalization.settings.appearance == "dark" ? NSAppearance(named: .darkAqua) : nil)
        files.configure(personalization.settings)
        index.searchEnabled = personalization.settings.appsEnabled
        index.refreshIfNeeded()
        mode = .root
        files.stop()
        input.stringValue = ""
        selection = 0
        message = nil
        updateResults()
        positionPanel()
        // Activation completes asynchronously. Focus now and again when AppKit
        // confirms activation, so a foreground editor cannot keep keyboard input.
        NSApp.activate(ignoringOtherApps: true)
        focusInput()
    }

    private func focusInput() {
        panel.makeKeyAndOrderFront(nil)
        if noteEditorSession != nil {
            let editor = noteEditorSession?.isEditing == true ? noteTextView(in: utilityHostingView) : nil
            panel.makeFirstResponder(editor ?? utilityHostingView)
            return
        }
        if mode == .dictation { panel.makeFirstResponder(utilityHostingView); return }
        // Re-entering an active field editor can replace text typed during activation.
        if let editor = input.currentEditor(), panel.firstResponder === editor { return }
        panel.makeFirstResponder(input)
    }

    private func noteTextView(in view: NSView?) -> MarkdownNoteTextView? {
        guard let view else { return nil }
        if let editor = view as? MarkdownNoteTextView { return editor }
        return view.subviews.lazy.compactMap { self.noteTextView(in: $0) }.first
    }

    func dismiss(restoreFocus: Bool = true) {
        guard panel.isVisible, !isConfirmingClear, saveNotes() else { return }
        inlineDictation?.cancel()
        cancelDictionaryLookup()
        actionsMenu.close(restoreFocus: false)
        panel.orderOut(nil)
        files.stop()
        if restoreFocus, let previousApp, previousApp.processIdentifier != ProcessInfo.processInfo.processIdentifier {
            previousApp.activate(options: [])
        }
    }
    func windowDidResignKey(_ notification: Notification) {
        if mode == .dictation, inlineDictation?.state == .requestingPermission { return }
        if !isConfirmingClear && !actionMenuOpen { dismiss(restoreFocus: false) }
    }
    func controlTextDidChange(_ obj: Notification) {
        setQuery(input.stringValue)
    }

    func setQuery(_ query: String) {
        if mode == .dictionary, input.stringValue != query || dictionaryLookedUp || dictionaryLoading {
            cancelDictionaryLookup()
            dictionaryDefinition = nil
            dictionaryLookedUp = false
        }
        if mode == .root, CalculatorEngine.currencyPair(in: query) != nil {
            Task { @MainActor [weak self] in
                await self?.currencyRates.refreshIfNeeded(for: query)
            }
        }
        if input.stringValue != query { input.stringValue = query }
        message = nil
        if mode == .root {
            if !personalization.settings.filesEnabled { files.stop() }
            else if case .result = calculator.evaluate(input.stringValue, currencyRates: currencyRates.snapshots) { files.stop() }
            else { files.search(input.stringValue) }
        }
        updateResults()
    }
    var previousApplication: NSRunningApplication? { previousApp }

    func pasteIntoPreviousApplication() {
        let target = previousApp
        dismiss(restoreFocus: false)
        pasteService.paste(into: target) { [weak self] failure in
            guard let failure else { return }
            self?.showPasteFeedback(failure.localizedDescription)
        }
    }

    private func showPasteFeedback(_ text: String) {
        let alert = NSAlert(); alert.messageText = L("Direct Paste"); alert.informativeText = text
        alert.addButton(withTitle: L("OK"))
        alert.runModal()
    }

    func restorePreviousFocus() {
        previousApp?.activate(options: [])
    }
    @objc private func goBack() {
        if mode == .localAssistant { localAssistant.cancel() }
        enter(.root)
    }
    private func enter(_ next: LauncherMode) {
        guard saveNotes() else { return }
        noteEditorSession = nil
        if mode == .dictation { inlineDictation?.cancel() }
        cancelDictionaryLookup()
        if next == .notes { connectNotes() }
        if next == .dictation, inlineDictation == nil { inlineDictation = OnDeviceDictationService() }
        dictionaryDefinition = nil
        dictionaryLookedUp = false
        utilityHostingView?.removeFromSuperview()
        utilityHostingView = nil
        mode = next
        input.stringValue = ""
        message = nil
        selection = 0
        files.stop()
        updateResults()
        positionPanel()
        if next == .dictation { panel.makeFirstResponder(utilityHostingView) }
        else { panel.makeFirstResponder(input) }
    }

    private func connectNotes() {
        guard loadedNotes == nil else { return }
        let store = noteStore?() ?? MarkdownNoteStore()
        loadedNotes = store
        notesSubscription = store.$notes.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in
            guard let self, self.mode == .notes, self.panel.isVisible else { return }
            if self.noteEditorSession == nil { self.updateResults(preserveSelection: true) }
        }
    }

    func createNote() {
        guard saveNotes() else { return }
        if !panel.isVisible { show() }
        enter(.notes)
        guard let store = loadedNotes, let note = store.create(contents: ""), store.note(id: note.id) != nil else {
            message = loadedNotes?.error
            updateResults()
            return
        }
        showNote(id: note.id)
    }

    func showNote(id: String, editing: Bool = true) {
        guard saveNotes() else { return }
        if !panel.isVisible { show() }
        enter(.notes)
        guard let store = loadedNotes, store.note(id: id) != nil else { return }
        let session = MarkdownNoteSession(store: store, onClose: { [weak self] in self?.dismiss() },
            onUnsavedDraftChanged: { [weak self] pending in self?.notesHasUnsavedDraft = pending })
        noteEditorSession = session
        session.open(initialNoteID: id, initiallyEditing: editing)
        render()
        positionPanel()
    }

    private func renderNotePreview() {
        notePreviewHostingView?.removeFromSuperview()
        let source = currentPreviewNoteID.flatMap { loadedNotes?.note(id: $0)?.contents }
        let view = AnyView(VStack(alignment: .leading, spacing: 10) {
            if let source {
                MarkdownPreview(source: source)
            } else {
                Text(L("New Note")).font(.headline)
                Text(L("Create a new Markdown note")).foregroundStyle(.secondary)
                Spacer()
            }
        }.padding(16).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading))
        let host = NSHostingView(rootView: view)
        host.translatesAutoresizingMaskIntoConstraints = false
        notePreviewHost.addSubview(host)
        NSLayoutConstraint.activate([
            host.leadingAnchor.constraint(equalTo: notePreviewHost.leadingAnchor),
            host.trailingAnchor.constraint(equalTo: notePreviewHost.trailingAnchor),
            host.topAnchor.constraint(equalTo: notePreviewHost.topAnchor),
            host.bottomAnchor.constraint(equalTo: notePreviewHost.bottomAnchor)
        ])
        notePreviewHostingView = host
    }

    private func previewVisibleNote() {
        guard mode == .notes, noteEditorSession == nil,
              rows.arrangedSubviews.indices.contains(selection) else { return }
        let visible = scrollView.contentView.bounds
        guard !visible.intersects(rows.arrangedSubviews[selection].frame),
              let next = rows.arrangedSubviews.indices.first(where: {
                  results.indices.contains($0) && visible.intersects(rows.arrangedSubviews[$0].frame)
              }) else { return }
        (rows.arrangedSubviews[selection] as? ResultButton)?.setSelected(false)
        selection = next
        (rows.arrangedSubviews[selection] as? ResultButton)?.setSelected(true)
        renderNotePreview()
    }

    private func cancelDictionaryLookup() {
        dictionaryRevision = UUID()
        dictionaryTask?.cancel()
        dictionaryTask = nil
        dictionaryLoading = false
    }

    func lookupDictionary() {
        guard mode == .dictionary else { return }
        cancelDictionaryLookup()
        let word = input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !word.isEmpty, word.utf16.count <= 256 else { return }
        let revision = dictionaryRevision
        let lookup = lookupDefinition
        dictionaryLoading = true
        dictionaryLookedUp = false
        render()
        dictionaryTask = Task { @MainActor [weak self] in
            let definition = await Task.detached { lookup(word) }.value
            guard !Task.isCancelled, let self, self.mode == .dictionary, self.dictionaryRevision == revision else { return }
            self.dictionaryDefinition = definition
            self.dictionaryLoading = false
            self.dictionaryLookedUp = true
            self.render()
        }
    }

    private func toggleDictation() {
        guard mode == .dictation, let service = inlineDictation else { return }
        if service.state == .recording { service.stop() }
        else if service.state == .idle { Task { await service.start() } }
    }

    private func insertDictation() {
        guard let service = inlineDictation, service.state == .idle else { return }
        let target = previousApp
        dismiss(restoreFocus: false)
        dictationDeliveryTask = Task { @MainActor [weak self] in
            if let failure = await service.insertTranscript(into: target), !Task.isCancelled {
                self?.showPasteFeedback(failure)
            }
        }
    }

    func handleUtilityKey(_ event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection([.command, .option, .control, .shift])
        if event.keyCode == 48, modifiers.isEmpty, !NSApp.isFullKeyboardAccessEnabled,
           mode == .localAssistant || mode == .chatHistory {
            showLocalAssistant(query: input.stringValue)
            return true
        }
        if mode == .localAssistant {
            if event.keyCode == 53 {
                if localAssistant.isResponding { localAssistant.cancel() }
                else { enter(.root) }
                return true
            }
            if [36, 76].contains(event.keyCode), modifiers.isEmpty,
               panel.firstResponder === input.currentEditor() || panel.firstResponder === input {
                submitLocalAssistantPrompt()
                return true
            }
            return false
        }
        if mode == .chatHistory, modifiers.isEmpty,
           panel.firstResponder === input.currentEditor() || panel.firstResponder === input {
            if [125, 126].contains(event.keyCode) {
                selection = max(0, min(matchingConversations.count - 1, selection + (event.keyCode == 125 ? 1 : -1)))
                renderUtility()
                return true
            }
            if [36, 76].contains(event.keyCode) {
                if matchingConversations.indices.contains(selection) {
                    resumeLocalAssistantConversation(id: matchingConversations[selection].id)
                }
                return true
            }
        }
        if event.keyCode == 53 {
            if mode == .dictation, inlineDictation?.state != .idle { inlineDictation?.cancel() }
            else { enter(.root) }
            return true
        }
        if mode == .dictionary, modifiers == .command,
           event.charactersIgnoringModifiers?.lowercased() == "c",
           ((panel.firstResponder as? NSTextView)?.selectedRange().length ?? 0) == 0,
           let definition = dictionaryDefinition {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(definition, forType: .string)
            return true
        }
        if mode == .dictionary, [36, 76].contains(event.keyCode), modifiers.isEmpty {
            lookupDictionary(); return true
        }
        if mode == .dictation, [36, 76].contains(event.keyCode) {
            if modifiers == [.command, .shift] { insertDictation(); return true }
            if modifiers == .command { toggleDictation(); return true }
        }
        return false
    }

    private func renderUtility() {
        for row in rows.arrangedSubviews { rows.removeArrangedSubview(row); row.removeFromSuperview() }
        category.stringValue = mode.title
        input.placeholderString = mode.placeholder
        searchIcon.isHidden = true
        backButton.isHidden = false
        clearHistoryButton.isHidden = true
        aiButton.isHidden = true
        actionButton.isHidden = true
        let view: AnyView
        if let session = noteEditorSession, let store = loadedNotes {
            if utilityHostingView != nil { resizeUtility(); return }
            view = AnyView(MarkdownNotesView(store: store, initialNoteID: session.selectedNoteID,
                initiallyEditing: session.isEditing, onBrowse: { [weak self] in self?.enter(.notes) },
                onActions: { [weak self] in self?.showActions() },
                onExport: { [weak self, weak session] in
                    guard let id = session?.selectedNoteID else { return }
                    self?.performNoteAction(.exportNote, id: id)
                }, onDelete: { [weak self, weak session] in
                    guard let id = session?.selectedNoteID else { return }
                    self?.performNoteAction(.deleteNote, id: id)
                }, session: session))
        } else if mode == .chatHistory {
            view = AnyView(LocalAssistantHistoryView(history: localAssistant.history, query: input.stringValue,
                selectedID: matchingConversations.indices.contains(selection) ? matchingConversations[selection].id : nil,
                onOpen: { [weak self] in self?.resumeLocalAssistantConversation(id: $0) },
                onDelete: { [weak self] in self?.removeLocalAssistantConversation(id: $0) }))
        } else if mode == .localAssistant {
            if utilityHostingView != nil { resizeUtility(); return }
            view = AnyView(LocalAssistantView(service: localAssistant, onSend: { [weak self] in
                self?.submitLocalAssistantPrompt()
            }))
        } else if mode == .dictionary {
            view = AnyView(LauncherDictionaryView(word: input.stringValue, definition: dictionaryDefinition,
                loading: dictionaryLoading, lookedUp: dictionaryLookedUp, onLookup: { [weak self] in self?.lookupDictionary() }))
        } else if let service = inlineDictation {
            if utilityHostingView != nil { resizeUtility(); return }
            view = AnyView(LauncherDictationView(service: service, application: previousApp, onInsert: { [weak self] in self?.insertDictation() }))
        } else { return }
        if mode == .chatHistory, let hosting = utilityHostingView as? NSHostingView<AnyView> {
            hosting.rootView = view
            resizeUtility()
            return
        }
        utilityHostingView?.removeFromSuperview()
        let hosting = NSHostingView(rootView: view)
        hosting.translatesAutoresizingMaskIntoConstraints = false
        utilityHost.addSubview(hosting)
        NSLayoutConstraint.activate([
            hosting.leadingAnchor.constraint(equalTo: utilityHost.leadingAnchor),
            hosting.trailingAnchor.constraint(equalTo: utilityHost.trailingAnchor),
            hosting.topAnchor.constraint(equalTo: utilityHost.topAnchor),
            hosting.bottomAnchor.constraint(equalTo: utilityHost.bottomAnchor)
        ])
        utilityHostingView = hosting
        resizeUtility()
    }

    private func resizeUtility() {
        let old = panel.frame
        panel.setFrame(NSRect(x: old.minX, y: old.maxY - 409, width: mode == .notes ? max(760, personalization.settings.panelWidth) : personalization.settings.panelWidth, height: 409), display: true)
    }

    private func updateResults(preserveSelection: Bool = false) {
        let interval = OSSignpostID(log: Self.performanceLog)
        os_signpost(.begin, log: Self.performanceLog, name: "Query results", signpostID: interval)
        defer { os_signpost(.end, log: Self.performanceLog, name: "Query results", signpostID: interval) }
        let selectedID = preserveSelection && results.indices.contains(selection) ? results[selection].id : nil
        let query = input.stringValue
        switch mode {
        case .root:
            let config = personalization.settings
            let appResults = config.appsEnabled ? index.applications.filter { !config.hiddenApps.contains($0.path) }.map(LauncherResult.application) : []
            let linkResults = config.linksEnabled ? quicklinks.links.map(LauncherResult.quicklink) : []
            let commands = config.commandsEnabled ? LauncherCommand.allCases.map(LauncherResult.command) : []
            let snippetResults = config.commandsEnabled ? snippets.snippets.map(LauncherResult.snippet) + scripts.commands.filter(\.enabled).map(LauncherResult.script) + extensions.manifests.filter(\.enabled).map(LauncherResult.userExtension) : []
            let shortcutResults = config.commandsEnabled ? appleShortcuts.selected.map(LauncherResult.appleShortcut) : []
            if query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                let rankedApps = rank(appResults, query: "")
                // Keep the empty panel bounded as the registry grows; search and Browse Commands expose everything.
                let suggestions: [LauncherResult] = config.commandsEnabled
                    ? [LauncherCommand.clipboard, .quicklinks, .snippets, .emoji, .notes, .calendar, .dictionary, .dictation, .chatHistory, .browse, .settings].map(LauncherResult.command)
                    : []
                let favorites = pinnedResults(from: appResults + linkResults + commands + shortcutResults + snippetResults)
                if config.emptyMode == "favorites" { results = favorites + suggestions.filter { !favorites.map(\.id).contains($0.id) } }
                else if config.emptyMode == "recent" { results = recentIDs.compactMap { id in (appResults + linkResults + shortcutResults).first { $0.id == id } } + suggestions }
                else { results = favorites + Array(rankedApps.filter { !favorites.map(\.id).contains($0.id) }.prefix(3)) + suggestions.filter { !favorites.map(\.id).contains($0.id) } }
            } else {
                results = Array(rank(appResults + linkResults + commands + shortcutResults + snippetResults, query: query).prefix(12))
                if LauncherUtilityQuery.dictionaryWord(in: query) != nil { results.insert(.command(.dictionary), at: 0); results = uniqueResults(results) }
                if LauncherUtilityQuery.noteSearch(in: query) != nil { results.insert(.command(.notes), at: 0); results = uniqueResults(results) }
                // Indexed files enrich results without delaying or overtaking app matches.
                results += files.results.filter { personalization.allowsFile(path: $0.path) }.prefix(12).map(LauncherResult.file)
                if config.linksEnabled, let resolved = quicklinks.resolve(query: query) { results.insert(.quicklink(resolved.link), at: 0); results = uniqueResults(results) }
                if config.linksEnabled, results.isEmpty, quicklinks.fallbackURL(query: query) != nil { results.append(.webSearch(query)) }
                switch calculator.evaluate(query, currencyRates: currencyRates.snapshots) {
                case .result(let result): results.insert(.calculation(result), at: 0)
                case .failure(let error): message = error
                case .none, .incomplete: break
                }
            }
        case .dictionary, .dictation, .localAssistant, .chatHistory: results = []
        case .notes:
            let matches = loadedNotes?.notes.filter { query.isEmpty || $0.title.localizedCaseInsensitiveContains(query) || $0.contents.localizedCaseInsensitiveContains(query) } ?? []
            results = matches.map(LauncherResult.note) + [.command(.newNote)]
        case .calculationHistory:
            results = calculationHistory.entries.filter { query.isEmpty || $0.expression.localizedCaseInsensitiveContains(query) || $0.display.localizedCaseInsensitiveContains(query) }.map(LauncherResult.historicalCalculation)
        case .snippets:
            results = rank(snippets.snippets.map(LauncherResult.snippet), query: query)
        case .webSearch:
            results = query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || quicklinks.fallbackURL(query: query) == nil ? [] : [.webSearch(query)]
        case .quicklinks:
            results = rank(quicklinks.links.map(LauncherResult.quicklink), query: query)
        case .clipboard:
            let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
            results = clipboard.items.filter { trimmed.isEmpty || $0.searchableText.localizedCaseInsensitiveContains(trimmed) }.map(LauncherResult.clipboard)
        }
        selection = selectedID.flatMap { id in results.firstIndex { $0.id == id } } ?? (preserveSelection ? min(selection, max(0, results.count - 1)) : 0)
        render()
        if !preserveSelection {
            announceSelection()
            scrollView.contentView.scroll(to: .zero)
            scrollView.reflectScrolledClipView(scrollView.contentView)
        }
    }

    private func rank(_ candidates: [LauncherResult], query: String) -> [LauncherResult] {
        let byID = Dictionary(uniqueKeysWithValues: candidates.map { ($0.id, $0) })
        let entries = candidates.map { result -> AppSearchEntry in
            let name: String
            switch result {
            case .application(let app): name = app.name
            case .script(let script): name = script.title
            case .userExtension(let manifest): name = manifest.name
            case .note(let note): name = note.title
            case .snippet(let snippet): name = snippet.name
            case .quicklink(let link): name = link.name
            case .command(let command): name = command.title
            case .appleShortcut(let shortcut): name = shortcut.name
            default: name = ""
            }
            let appAliases: [String]
            if case .application(let app) = result, let alias = aliases.alias(for: app) { appAliases = [alias] }
            else if case .command(let command) = result { appAliases = command.aliases }
            else { appAliases = [] }
            return AppSearchEntry(id: result.id, name: name, path: "", aliases: appAliases)
        }
        return search.ranked(query: query, entries: entries, usage: usage).compactMap { byID[$0.id] }
    }

    func moveSelection(_ delta: Int) {
        guard !results.isEmpty else { return }
        let previous = selection
        selection = min(max(0, selection + delta), results.count - 1)
        guard previous != selection else { return }
        (rows.arrangedSubviews[previous] as? ResultButton)?.setSelected(false)
        (rows.arrangedSubviews[selection] as? ResultButton)?.setSelected(true)
        if mode == .notes { renderNotePreview() }
        rows.layoutSubtreeIfNeeded()
        if rows.arrangedSubviews.indices.contains(selection) {
            rows.scrollToVisible(rows.arrangedSubviews[selection].frame)
            announceSelection()
        }
    }

    private func render() {
        let interval = OSSignpostID(log: Self.performanceLog)
        os_signpost(.begin, log: Self.performanceLog, name: "Render rows", signpostID: interval)
        defer { os_signpost(.end, log: Self.performanceLog, name: "Render rows", signpostID: interval) }
        let isUtility = mode == .dictionary || mode == .dictation || mode == .localAssistant || mode == .chatHistory || noteEditorSession != nil
        utilityHost.isHidden = !isUtility
        resultsBody.isHidden = isUtility
        notePreviewHost.isHidden = mode != .notes
        scrollView.isHidden = isUtility
        input.isEditable = mode != .dictation && noteEditorSession == nil
        input.isSelectable = mode != .dictation && noteEditorSession == nil
        if isUtility { renderUtility(); return }
        utilityHostingView?.removeFromSuperview()
        utilityHostingView = nil
        for view in rows.arrangedSubviews { rows.removeArrangedSubview(view); view.removeFromSuperview() }
        category.stringValue = mode.title + (profileName.map { " · " + $0() } ?? "")
        actionButton.isHidden = false
        actionButton.isEnabled = !results.isEmpty
        actionButton.title = personalization.settings.showShortcutHints ? "Actions ⌘K" : "Actions"
        input.placeholderString = mode.placeholder
        searchIcon.isHidden = mode != .root
        backButton.isHidden = mode == .root
        clearHistoryButton.isHidden = mode != .clipboard
        clearHistoryButton.isEnabled = !clipboard.items.isEmpty
        aiButton.isHidden = mode != .root || (ai.provider != .local && input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        aiButton.title = "Ask \(ai.provider.title)" + (personalization.settings.showShortcutHints ? " ⇥" : "")
        aiButton.setAccessibilityLabel(ai.provider == .local ? "Chat with Local Assistant" : "Open \(ai.provider.title) with your query")
        aiButton.toolTip = "Send query to \(ai.provider.title) (Tab)"
        if results.isEmpty {
            let label = NSTextField(labelWithString: message ?? emptyMessage)
            label.font = .systemFont(ofSize: 14)
            label.textColor = .secondaryLabelColor
            label.alignment = .center
            rows.addArrangedSubview(label)
            label.heightAnchor.constraint(equalToConstant: 48).isActive = true
            label.widthAnchor.constraint(equalTo: rows.widthAnchor, constant: -12).isActive = true
        }
        for (position, result) in results.enumerated() {
            let button = ResultButton()
            button.wantsLayer = true
            button.layer?.cornerRadius = 10
            button.layer?.backgroundColor = position == selection ? NSColor.controlAccentColor.withAlphaComponent(0.14).cgColor : NSColor.clear.cgColor
            let icon: NSImage?
            let title: String
            let subtitle: String
            let action: String
            switch result {
            case .application(let app):
                icon = index.icon(for: app) ?? NSImage(systemSymbolName: "app", accessibilityDescription: nil)
                title = app.name
                let hasDuplicateName = index.applications.contains { $0.id != app.id && $0.name == app.name }
                subtitle = (hasDuplicateName ? NSString(string: app.path).abbreviatingWithTildeInPath : L("Application")) + (ApplicationControlService.isRunning(app) ? " · " + L("Running") : "")
                action = L("Open")
            case .command(let command):
                icon = NSImage(systemSymbolName: command.symbol, accessibilityDescription: nil)
                title = command.title
                switch command {
                case .dictionary: subtitle = LauncherUtilityQuery.dictionaryWord(in: input.stringValue) ?? L("Type a word, then press Return…")
                case .notes: subtitle = LauncherUtilityQuery.noteSearch(in: input.stringValue) ?? L("Browse notes · ⌘N New Note")
                case .dictation: subtitle = L("On-device dictation · ⌘Return Start or stop")
                case .settings: subtitle = L("Preferences and shortcuts")
                case .shutdown: subtitle = L("Shows macOS shutdown confirmation")
                default: subtitle = L("Command")
                }
                action = L("Open")
            case .appleShortcut(let shortcut):
                icon = NSImage(systemSymbolName: "square.stack.3d.up", accessibilityDescription: nil)
                title = shortcut.name; subtitle = L("Apple Shortcut"); action = L("Run")
            case .webSearch(let query):
                icon = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)
                title = "Search the web for \(query)"; subtitle = L("Opens your configured search website"); action = L("Search")
            case .quicklink(let link):
                icon = NSImage(systemSymbolName: "link", accessibilityDescription: nil)
                title = link.name
                let argument = quicklinks.resolve(query: input.stringValue).flatMap { $0.link.id == link.id ? $0.argument : nil } ?? ""
                subtitle = link.keyword.isEmpty ? link.target : "\(link.keyword) · \(link.resolvedURL(argument: argument)?.absoluteString ?? link.target)"
                action = L("Open")
            case .note(let note):
                icon = NSImage(systemSymbolName: "note.text", accessibilityDescription: nil)
                title = note.title
                subtitle = note.modifiedAt.formatted(date: .abbreviated, time: .shortened)
                action = L("Edit")
            case .script(let script):
                icon = NSImage(systemSymbolName: "terminal", accessibilityDescription: nil)
                title = script.title; subtitle = script.description; action = L("Run")
            case .userExtension(let manifest):
                icon = NSImage(systemSymbolName: "puzzlepiece.extension", accessibilityDescription: nil)
                title = manifest.name; subtitle = manifest.description; action = L("Open")
            case .snippet(let snippet):
                icon = NSImage(systemSymbolName: "text.quote", accessibilityDescription: nil)
                title = snippet.name; subtitle = String(snippet.template.prefix(120)); action = L("Preview")
            case .clipboard(let entry):
                icon = NSImage(systemSymbolName: "clipboard", accessibilityDescription: nil)
                title = (entry.isPinned ? "📌 " : "") + entry.preview
                subtitle = entry.date.formatted(date: .abbreviated, time: .shortened)
                action = L("Paste")
            case .file(let file):
                icon = NSImage(systemSymbolName: "doc", accessibilityDescription: nil)
                title = file.name
                subtitle = NSString(string: file.path).abbreviatingWithTildeInPath
                action = L("Open")
            case .historicalCalculation(let entry):
                icon = NSImage(systemSymbolName: "clock.arrow.circlepath", accessibilityDescription: nil)
                title = entry.display; subtitle = entry.expression + " · " + entry.acceptedAt.formatted(date: .abbreviated, time: .shortened); action = L("Copy Answer")
            case .calculation(let result):
                icon = NSImage(systemSymbolName: "equal.square", accessibilityDescription: nil)
                title = result.display
                subtitle = result.expression
                action = L("Copy answer")
            }
            let image = NSImageView()
            image.image = icon
            if case .clipboard(let entry) = result, entry.imageAttachment != nil {
                Task { [weak image, weak clipboard] in
                    if let data = await clipboard?.imageThumbnailData(for: entry) { image?.image = NSImage(data: data) }
                }
            }
            image.imageScaling = .scaleProportionallyUpOrDown
            let titleLabel = NSTextField(labelWithString: title)
            titleLabel.font = .systemFont(ofSize: personalization.settings.textSize, weight: .medium)
            titleLabel.lineBreakMode = .byTruncatingTail
            let subtitleLabel = NSTextField(labelWithString: subtitle)
            subtitleLabel.font = .systemFont(ofSize: max(10, personalization.settings.textSize - 4))
            subtitleLabel.textColor = .secondaryLabelColor
            subtitleLabel.lineBreakMode = .byTruncatingMiddle
            let hint = NSTextField(labelWithString: position == selection && personalization.settings.showShortcutHints ? "↵" : "")
            hint.textColor = .secondaryLabelColor
            button.selectionHint = hint
            button.showsHint = personalization.settings.showShortcutHints
            for view in [image, titleLabel, subtitleLabel, hint] { view.translatesAutoresizingMaskIntoConstraints = false; button.addSubview(view) }
            let isFavorite = personalization.isPinned(id: result.id)
            var titleTrailingAnchor = hint.leadingAnchor
            if isFavorite {
                let pinImage = NSImageView()
                pinImage.identifier = NSUserInterfaceItemIdentifier("favorite-pin")
                pinImage.image = NSImage(systemSymbolName: "pin.fill", accessibilityDescription: nil)
                pinImage.contentTintColor = .secondaryLabelColor
                pinImage.toolTip = L("Pinned to Favorites")
                pinImage.setAccessibilityElement(false)
                pinImage.translatesAutoresizingMaskIntoConstraints = false
                button.addSubview(pinImage)
                NSLayoutConstraint.activate([
                    pinImage.widthAnchor.constraint(equalToConstant: 12),
                    pinImage.heightAnchor.constraint(equalToConstant: 12),
                    pinImage.centerYAnchor.constraint(equalTo: titleLabel.centerYAnchor),
                    pinImage.trailingAnchor.constraint(equalTo: hint.leadingAnchor, constant: -12)
                ])
                titleTrailingAnchor = pinImage.leadingAnchor
            }
            NSLayoutConstraint.activate([
                button.heightAnchor.constraint(equalToConstant: rowHeight),
                image.leadingAnchor.constraint(equalTo: button.leadingAnchor, constant: 10),
                image.centerYAnchor.constraint(equalTo: button.centerYAnchor),
                image.widthAnchor.constraint(equalToConstant: 28), image.heightAnchor.constraint(equalToConstant: 28),
                titleLabel.leadingAnchor.constraint(equalTo: image.trailingAnchor, constant: 10),
                titleLabel.topAnchor.constraint(equalTo: button.topAnchor, constant: 6),
                titleLabel.trailingAnchor.constraint(lessThanOrEqualTo: titleTrailingAnchor, constant: -12),
                subtitleLabel.leadingAnchor.constraint(equalTo: titleLabel.leadingAnchor),
                subtitleLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 1),
                subtitleLabel.trailingAnchor.constraint(lessThanOrEqualTo: titleTrailingAnchor, constant: -12),
                hint.trailingAnchor.constraint(equalTo: button.trailingAnchor, constant: isClipboardResult(result) ? -40 : -10),
                hint.centerYAnchor.constraint(equalTo: button.centerYAnchor)
            ])
            if case .clipboard(let entry) = result {
                let remove = ResultButton()
                remove.image = NSImage(systemSymbolName: "trash", accessibilityDescription: "Remove clipboard item")
                remove.contentTintColor = .secondaryLabelColor
                remove.translatesAutoresizingMaskIntoConstraints = false
                remove.setAccessibilityLabel(L("Remove clipboard item"))
                remove.toolTip = "Remove item (⌘⌫)"
                remove.invoke = { [weak self] in self?.clipboard.remove(id: entry.id) }
                button.addSubview(remove)
                button.setAccessibilityChildren([remove])
                NSLayoutConstraint.activate([
                    remove.trailingAnchor.constraint(equalTo: button.trailingAnchor, constant: -8),
                    remove.centerYAnchor.constraint(equalTo: button.centerYAnchor),
                    remove.widthAnchor.constraint(equalToConstant: 24),
                    remove.heightAnchor.constraint(equalToConstant: 28)
                ])
            }
            button.setAccessibilityLabel("\(title), \(subtitle), \(action)" + (isFavorite ? ", " + L("Pinned to Favorites") : ""))
            button.setSelected(position == selection)
            button.contextInvoke = { [weak self] in
                guard let self, let position = self.results.firstIndex(where: { $0.id == result.id }) else { return }
                self.selection = position; if self.mode == .notes { self.renderNotePreview() }; self.showActions()
            }
            button.invoke = { [weak self] in
                guard let self, let current = self.results.firstIndex(where: { $0.id == result.id }) else { return }
                self.selection = current
                if case .note = result { self.render(); self.focusInput() }
                else { self.performSelected() }
            }
            rows.addArrangedSubview(button)
            button.widthAnchor.constraint(equalTo: rows.widthAnchor, constant: -12).isActive = true
        }
        if let message, !results.isEmpty {
            let errorLabel = NSTextField(wrappingLabelWithString: message)
            errorLabel.font = .systemFont(ofSize: 12)
            errorLabel.textColor = .secondaryLabelColor
            rows.addArrangedSubview(errorLabel)
            errorLabel.heightAnchor.constraint(equalToConstant: 40).isActive = true
            errorLabel.widthAnchor.constraint(equalTo: rows.widthAnchor, constant: -12).isActive = true
        }
        // Keep the top edge fixed while the result area expands or contracts.
        let resultHeight: Int = results.isEmpty ? 48 : Int(Double(results.count) * (rowHeight + 2)) - 2
        let errorHeight = message != nil && !results.isEmpty ? 42 : 0
        let documentHeight = CGFloat(resultHeight + errorHeight + 8)
        let visibleHeight = mode == .notes ? 330 : min(documentHeight, 330)
        let panelWidth = mode == .notes ? max(760, personalization.settings.panelWidth) : personalization.settings.panelWidth
        rows.setFrameSize(NSSize(width: mode == .notes ? panelWidth * 0.45 : panelWidth, height: documentHeight))
        scrollHeight.constant = visibleHeight
        let height = CGFloat(79) + visibleHeight
        let old = panel.frame
        panel.setFrame(NSRect(x: old.minX, y: old.maxY - height, width: panelWidth, height: height), display: true)
        if mode == .notes { renderNotePreview() }
    }

    private var rowHeight: Double { max(personalization.settings.compact ? 40 : 48, personalization.settings.textSize * 2 + 16) }

    private func uniqueResults(_ candidates: [LauncherResult]) -> [LauncherResult] {
        var seen = Set<String>()
        return candidates.filter { seen.insert($0.id).inserted }
    }

    private func pinnedResults(from candidates: [LauncherResult]) -> [LauncherResult] {
        personalization.pins.compactMap { pin in
            if let result = candidates.first(where: { $0.id == pin.id }) { return result }
            if pin.kind == "file", personalization.settings.filesEnabled, personalization.allowsFile(path: pin.path), FileManager.default.fileExists(atPath: pin.path) {
                return .file(FileSearchResult(name: pin.name, path: pin.path))
            }
            return nil
        }
    }

    private func recordRecent(_ id: String) {
        recentIDs.removeAll { $0 == id }; recentIDs.insert(id, at: 0)
        recentIDs = Array(recentIDs.prefix(20))
        UserDefaults.standard.set(recentIDs, forKey: "recentResults")
    }

    private func announceSelection() {
        guard mode != .dictionary && mode != .dictation else { return }
        guard rows.arrangedSubviews.indices.contains(selection) else { return }
        let row = rows.arrangedSubviews[selection]
        NSAccessibility.post(element: input, notification: .announcementRequested, userInfo: [
            .announcement: row.accessibilityLabel() ?? "", .priority: NSAccessibilityPriorityLevel.medium.rawValue
        ])
    }

    // Use the same live stores and enabled flags as the launcher. Never let the
    // model invent an executable, application path, or disabled command.
    func assistantCatalog(query: String) -> String {
        struct Action: Encodable {
            let id: String
            let title: String
            var application: AssistantApplicationContext? = nil
        }
        var entries: [(String, String)] = []
        let config = personalization.settings
        if config.commandsEnabled {
            entries += LauncherCommand.allCases.map { ($0.id, $0.title) }
            entries += scripts.commands.filter(\.enabled).map { ($0.commandID, $0.title + " (input: optional JSON array of argument strings)") }
            entries += extensions.manifests.filter(\.enabled).map { ($0.commandID, $0.name) }
            entries += snippets.snippets.map { ("snippet:" + $0.id, $0.name) }
            entries += appleShortcuts.selected.map { ("apple:" + $0.id, $0.name) }
            entries += [("calculate", "Calculate an expression; input is the expression"),
                        ("dictionary.lookup", "Look up a word; input is the word"),
                        ("notes.search", "Search note contents; input is the search text"),
                        ("notes.create", "Create a Markdown note; input is its full content")]
        }
        if config.linksEnabled { entries += quicklinks.links.map { ("link:" + $0.id, $0.name) } }
        // Supply only relevant app identities; the local model has a small context.
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let ignoredWords: Set<String> = ["open", "launch", "run", "start", "please", "can", "you", "the", "app", "application"]
        let words = query.lowercased().split(whereSeparator: \.isWhitespace).map(String.init).filter { !ignoredWords.contains($0) }
        let matches = entries.filter { entry in
            query.isEmpty || words.allSatisfy { entry.0.localizedCaseInsensitiveContains($0) || entry.1.localizedCaseInsensitiveContains($0) }
        }
        var actions = matches.prefix(30).map { Action(id: $0.0, title: $0.1) }
        if config.appsEnabled, !query.isEmpty {
            actions += assistantApplicationCandidates(query: words.joined(separator: " ")).map { app in
                Action(id: "app:" + app.id, title: app.name,
                       application: AssistantApplicationContext(app: app, customAlias: aliases.alias(for: app)))
            }
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = .withoutEscapingSlashes
        while !actions.isEmpty {
            if let data = try? encoder.encode(actions), data.count <= 5_500 { return String(decoding: data, as: UTF8.self) }
            actions.removeLast()
        }
        return "[]"
    }

    private func assistantApplicationCandidates(query: String) -> [IndexedApplication] {
        guard personalization.settings.appsEnabled else { return [] }
        let available = index.applications.filter { !personalization.settings.hiddenApps.contains($0.path) }
        let entries = available.map { app in
            AppSearchEntry(id: app.id, name: app.name, path: app.path,
                           aliases: AssistantApplicationIdentity(app: app, customAlias: aliases.alias(for: app)).aliases
                            + [app.bundleIdentifier].compactMap { $0 })
        }
        let ignoredWords: Set<String> = ["open", "launch", "run", "start", "please", "can", "you", "the", "app", "application"]
        let terms = query.split(whereSeparator: \.isWhitespace).filter { !ignoredWords.contains($0.lowercased()) }.joined(separator: " ")
        let ranked = search.ranked(query: terms, entries: entries)
        let byID = Dictionary(uniqueKeysWithValues: available.map { ($0.id, $0) })
        return ranked.prefix(10).compactMap { byID[$0.id] }
    }

    private func assistantApplicationContext(query: String) -> String {
        var context = assistantApplicationCandidates(query: query).map {
            AssistantApplicationContext(app: $0, customAlias: aliases.alias(for: $0))
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = .withoutEscapingSlashes
        while !context.isEmpty {
            if let data = try? encoder.encode(context), data.count <= 5_500 { return String(decoding: data, as: UTF8.self) }
            context.removeLast()
        }
        return "[]"
    }

    func executeAssistantAction(id: String, input text: String) async throws -> String {
        try Task.checkCancellation()
        let config = personalization.settings
        if id == "application.search" {
            guard config.appsEnabled else { throw AutomationFailure.invalid("Application search is disabled in Settings.") }
            return assistantApplicationContext(query: text)
        }
        if id == "application.open" {
            guard config.appsEnabled else { throw AutomationFailure.invalid("Application search is disabled in Settings.") }
            let name = text.trimmingCharacters(in: .whitespacesAndNewlines)
                .replacingOccurrences(of: #"\.app$"#, with: "", options: [.regularExpression, .caseInsensitive])
            guard !name.isEmpty else { throw AutomationFailure.invalid("Provide an application name or alias.") }
            let available = index.applications.filter { !config.hiddenApps.contains($0.path) }
            let matches = AssistantApplicationIdentity.resolve(query: name, applications: available, customAliases: aliases.aliases)
            guard matches.count == 1, let app = matches.first else {
                let candidates = matches.isEmpty ? assistantApplicationCandidates(query: name) : matches
                let context = candidates.prefix(5).map { app in
                    "\(app.name) (\(app.path)); aliases: " + AssistantApplicationIdentity(app: app, customAlias: aliases.alias(for: app)).aliases.joined(separator: ", ")
                }.joined(separator: "\n")
                throw AutomationFailure.invalid((matches.isEmpty ? "No exact application identity matches \(name)." : "Multiple applications match \(name). Ask which one the user means.")
                    + " Use find_applications for installed app names, paths, bundle identities, and aliases.\n" + context)
            }
            return try await executeAssistantAction(id: "app:" + app.id, input: "")
        }
        if config.appsEnabled, id.hasPrefix("app:"),
           let app = index.applications.first(where: { "app:" + $0.id == id && !config.hiddenApps.contains($0.path) }) {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                launchApplication(app) { error in
                    if let error { continuation.resume(throwing: error) }
                    else { continuation.resume() }
                }
            }
            return "Opened \(app.name)."
        }
        if config.linksEnabled, let link = quicklinks.links.first(where: { "link:" + $0.id == id }) {
            guard let url = link.resolvedURL(argument: text), NSWorkspace.shared.open(url) else {
                throw AutomationFailure.invalid("Could not open this quicklink.")
            }
            return "Opened \(link.name)."
        }
        guard config.commandsEnabled else { throw AutomationFailure.invalid("Kebloop commands are disabled in Settings.") }
        switch id {
        case "calculate":
            await currencyRates.refreshIfNeeded(for: text)
            guard case .result(let result) = calculator.evaluate(text, currencyRates: currencyRates.snapshots) else {
                throw AutomationFailure.invalid("Could not calculate that expression.")
            }
            return result.clipboard
        case "dictionary.lookup":
            let lookup = lookupDefinition
            return await Task.detached { lookup(text) ?? "No definition found." }.value
        case "notes.search":
            connectNotes()
            return String((loadedNotes?.notes.filter {
                text.isEmpty || $0.title.localizedCaseInsensitiveContains(text) || $0.contents.localizedCaseInsensitiveContains(text)
            }.prefix(5).map { "\($0.title)\n\($0.contents.prefix(1000))" }.joined(separator: "\n\n") ?? "").prefix(5000))
        case "notes.create":
            connectNotes()
            guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  let note = loadedNotes?.create(contents: text) else {
                throw AutomationFailure.invalid(loadedNotes?.error ?? "Provide note content.")
            }
            return "Created note: \(note.title)"
        default: break
        }
        if let script = scripts.commands.first(where: { $0.commandID == id && $0.enabled }) {
            let arguments: [String]
            if text.isEmpty { arguments = [] }
            else if let parsed = try? JSONDecoder().decode([String].self, from: Data(text.utf8)),
                    parsed.allSatisfy({ !$0.contains("\0") }) { arguments = parsed }
            else { throw AutomationFailure.invalid("Script input must be a JSON array of argument strings, or empty.") }
            try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                    scripts.run(id: script.id, arguments: arguments) { error in
                        if let error { continuation.resume(throwing: AutomationFailure.invalid(error)) }
                        else { continuation.resume() }
                    }
                }
            } onCancel: { [weak self] in Task { @MainActor in self?.scripts.cancel() } }
            return String((scripts.output?.stdout ?? "Script completed.").prefix(5000))
        }
        if let shortcut = appleShortcuts.selected.first(where: { "apple:" + $0.id == id }) {
            try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                    appleShortcuts.run(id: shortcut.id, input: text.isEmpty ? nil : text) { error in
                        if let error { continuation.resume(throwing: AutomationFailure.invalid(error)) }
                        else { continuation.resume() }
                    }
                }
            } onCancel: { [weak self] in Task { @MainActor in self?.appleShortcuts.cancel() } }
            return "Completed Shortcut: \(shortcut.name).\n" + String((appleShortcuts.output ?? "").prefix(5000))
        }
        if let command = LauncherCommand(rawValue: id) {
            if let action = command.systemAction {
                try await SystemActionService.perform(action)
                return "Requested \(command.title). Any macOS confirmation still requires the user."
            }
            if let action = command.windowAction {
                try windowManagement.perform(action, application: previousApp)
                return "Completed \(command.title)."
            }
            if command == .shutdown {
                try ShutdownService.requestConfirmation()
                return "Opened the macOS shutdown confirmation. The user must confirm."
            }
            executeCommand(id: id)
            if !text.isEmpty, [.dictionary, .notes, .webSearch, .clipboard, .snippets, .quicklinks].contains(command) { setQuery(text) }
            return "Opened \(command.title) workflow; the user can continue there."
        }
        if let manifest = extensions.manifests.first(where: { $0.commandID == id && $0.enabled }) {
            onExtension?(manifest)
            return "Opened \(manifest.name) extension workflow; the user can continue there."
        }
        if snippets.snippets.contains(where: { "snippet:" + $0.id == id }) {
            executeCommand(id: id)
            return "Opened the snippet workflow."
        }
        throw AutomationFailure.invalid("Unknown or disabled action. Search the action catalog for its exact ID.")
    }

    func executeCommand(id: String) {
        guard personalization.settings.commandsEnabled else { return }
        guard saveNotes() else { focusInput(); return }
        if let script = scripts.commands.first(where: { $0.commandID == id && $0.enabled }) { onScript?(script, []); return }
        if let manifest = extensions.manifests.first(where: { $0.commandID == id && $0.enabled }) { onExtension?(manifest); return }
        if id.hasPrefix("snippet:"), let snippet = snippets.snippets.first(where: { "snippet:" + $0.id == id }) {
            show(); results = [.snippet(snippet)]; selection = 0; performSelected(); return
        }
        if id.hasPrefix("apple:"), let shortcut = appleShortcuts.selected.first(where: { "apple:" + $0.id == id }) {
            show(); results = [.appleShortcut(shortcut)]; selection = 0; performSelected(); return
        }
        if id.hasPrefix("link:"), let link = quicklinks.links.first(where: { "link:" + $0.id == id }) {
            show(); results = [.quicklink(link)]; selection = 0; performSelected(); return
        }
        guard let command = LauncherCommand(rawValue: id.replacingOccurrences(of: "command:", with: "")) else { return }
        if let action = command.windowAction {
            let frontmost = NSWorkspace.shared.frontmostApplication
            let target = frontmost?.processIdentifier == ProcessInfo.processInfo.processIdentifier ? previousApp : frontmost
            do {
                try windowManagement.perform(action, application: target)
                if panel.isVisible { dismiss(restoreFocus: false) }
                target?.activate(options: [])
            } catch {
                show()
                results = [.command(command)]
                selection = 0
                message = error.localizedDescription
                render()
            }
            return
        }
        show(); results = [.command(command)]; selection = 0; performSelected()
    }

    private func applyPersonalization() {
        let config = personalization.settings
        panel.appearance = config.appearance == "light" ? NSAppearance(named: .aqua) : (config.appearance == "dark" ? NSAppearance(named: .darkAqua) : nil)
        buildInterface()
        index.searchEnabled = config.appsEnabled
        files.configure(config)
        if !config.filesEnabled { files.stop() }
        else if panel.isVisible { files.search(input.stringValue) }
        if panel.isVisible { updateResults(preserveSelection: true); positionPanel() }
    }

    private func positionPanel() {
        let config = personalization.settings
        let pointer = NSScreen.screens.first { NSMouseInRect(NSEvent.mouseLocation, $0.frame, false) }
        let chosen = NSScreen.screens.first { String(describing: $0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] ?? "") == config.displayID }
        let screen = config.placement == "display" ? (chosen ?? pointer ?? NSScreen.main) : (config.placement == "active" ? (activeScreen ?? NSScreen.main ?? pointer) : (pointer ?? NSScreen.main))
        guard let screen else { return }
        let area = screen.visibleFrame
        var frame = panel.frame
        frame.size.width = min(mode == .notes ? max(760, config.panelWidth) : config.panelWidth, area.width)
        frame.size.height = min(frame.height, area.height)
        frame.origin.x = max(area.minX, min(area.maxX - frame.width, area.midX - frame.width / 2))
        frame.origin.y = max(area.minY, min(area.maxY - frame.height, area.maxY - max(80, area.height * 0.2) - frame.height))
        panel.setFrame(frame, display: true)
    }

    private func pin(for result: LauncherResult) -> PinnedResult? {
        switch result {
        case .application(let app): return PinnedResult(id: result.id, name: app.name, path: app.path, kind: "application")
        case .file(let file): return PinnedResult(id: result.id, name: file.name, path: file.path, kind: "file")
        case .quicklink(let link): return PinnedResult(id: result.id, name: link.name, path: link.target, kind: "quicklink")
        case .command(let command): return PinnedResult(id: result.id, name: command.title, path: "", kind: "command")
        default: return nil
        }
    }

    private func actionItems(for result: LauncherResult) -> [ActionMenuItem] {
        var items: [ActionMenuItem] = []
        func add(_ action: ResultAction) {
            let title: String
            if action == .open {
                switch result {
                case .clipboard: title = L("Paste")
                case .calculation, .historicalCalculation: title = L("Copy Answer")
                case .appleShortcut: title = L("Run")
                case .webSearch: title = L("Search")
                default: title = action.title
                }
            } else { title = action.title }
            items.append(ActionMenuItem(action: action, title: title))
        }
        let kind: ResultAction.Kind
        switch result {
        case .application: kind = .application
        case .file: kind = .file
        case .quicklink(let link): kind = quicklinks.links.contains { $0.id == link.id } ? .quicklink : .other
        case .command: kind = .command
        case .appleShortcut: kind = .appleShortcut
        default: kind = .other
        }
        if mode == .calculationHistory {
            add(.open); add(.clearCalculationHistory); return items
        }
        if case .note = result {
            for action in [ResultAction.editNote, .copyMarkdown, .exportNote, .deleteNote] { add(action) }
            return items
        }
        if case .script = result { add(.open); add(.runWithInput); return items }
        if case .clipboard(let entry) = result {
            for action in ResultAction.available(kind: .clipboard, pinned: entry.isPinned, hasPlainText: !entry.text.isEmpty) {
                if action == .pin || action == .unpin {
                    items.append(ActionMenuItem(action: action, title: entry.isPinned ? L("Unpin Clipboard Item") : L("Pin Clipboard Item")))
                } else { add(action) }
            }
            return items
        }
        let running: Bool
        if case .application(let app) = result { running = ApplicationControlService.isRunning(app) } else { running = false }
        for action in ResultAction.available(kind: kind, pinned: personalization.isPinned(id: result.id), running: running) { add(action) }
        return items
    }

    @objc func showActions() {
        if actionsMenu.isVisible { actionsMenu.close(); return }
        let result: LauncherResult
        if let note = noteEditorSession?.selectedNote { result = .note(note) }
        else {
            guard results.indices.contains(selection) else { return }
            result = results[selection]
        }
        let title = noteEditorSession?.selectedNote?.title ?? (rows.arrangedSubviews.indices.contains(selection) ? rows.arrangedSubviews[selection].accessibilityLabel() ?? mode.title : mode.title)
        actionsMenu.show(items: actionItems(for: result), title: title, parent: panel,
                         opaque: !personalization.settings.transparency || NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency,
                         onChoose: { [weak self] action in self?.performResultAction(action, resultID: result.id) },
                         onClose: { [weak self] restoreFocus in
            guard let self, self.panel.isVisible else { return }
            if restoreFocus { self.focusInput(); self.announceSelection() }
            else {
                DispatchQueue.main.async { [weak self] in
                    guard let self, self.panel.isVisible, !self.panel.isKeyWindow, !self.actionsMenu.isVisible else { return }
                    self.dismiss(restoreFocus: false)
                }
            }
        })
    }

    var isShowingActions: Bool { actionsMenu.isVisible }
    var visibleActionIdentifiers: [ResultAction] { actionsMenu.items.map(\.action) }
    @discardableResult
    func handleActionKey(_ event: NSEvent) -> Bool { actionsMenu.handleKey(event) }

    private func performResultAction(_ action: ResultAction, resultID: String) {
        if resultID.hasPrefix("note:"), [.editNote, .copyMarkdown, .exportNote, .deleteNote].contains(action) {
            performNoteAction(action, id: String(resultID.dropFirst(5)))
            return
        }
        guard let position = results.firstIndex(where: { $0.id == resultID }) else {
            message = L("This result is no longer available."); render(); return
        }
        selection = position
        let result = results[position]
        let path: String?
        switch result { case .file(let file): path = file.path; case .application(let app): path = app.path; default: path = nil }
        switch action {
        case .editNote, .copyMarkdown, .exportNote, .deleteNote: break
        case .removeClipboardItem:
            guard case .clipboard(let entry) = result else { return }
            clipboard.remove(id: entry.id)
            updateResults(preserveSelection: true)
        case .clearCalculationHistory:
            let alert = NSAlert(); alert.messageText = L("Clear calculator history?")
            alert.informativeText = L("All accepted calculations will be removed.")
            alert.addButton(withTitle: L("Clear History")); alert.addButton(withTitle: L("Cancel"))
            alert.buttons[0].hasDestructiveAction = true
            isConfirmingClear = true; let response = alert.runModal(); isConfirmingClear = false
            if response == .alertFirstButtonReturn { calculationHistory.clear(); updateResults() }
            focusInput()
        case .copy:
            guard case .clipboard(let entry) = result else { return }
            clipboard.copy(entry); dismiss()
        case .pastePlainText:
            guard case .clipboard(let entry) = result, !entry.text.isEmpty else { return }
            NSPasteboard.general.clearContents(); NSPasteboard.general.setString(entry.text, forType: .string)
            pasteIntoPreviousApplication()
        case .quitApplication, .forceQuitApplication, .restartApplication, .hideApplication:
            guard case .application(let app) = result else { return }
            if action == .forceQuitApplication {
                let alert = NSAlert(); alert.messageText = L("Force Quit Application…")
                alert.informativeText = app.name + "\n" + L("Unsaved changes may be lost.")
                alert.addButton(withTitle: L("Force Quit")); alert.addButton(withTitle: L("Cancel"))
                alert.buttons[0].hasDestructiveAction = true
                isConfirmingClear = true; let response = alert.runModal(); isConfirmingClear = false
                guard response == .alertFirstButtonReturn else { return }
            }
            let operation: ApplicationControlService.Action
            switch action {
            case .forceQuitApplication: operation = .forceQuit
            case .restartApplication: operation = .restart
            case .hideApplication: operation = .hide
            default: operation = .quit
            }
            let context = beginExternalAction()
            ApplicationControlService.perform(operation, for: app) { [weak self] error in
                if let error { self?.showExternalActionError(error.localizedDescription, context: context) }
            }
        case .open: performSelected()
        case .runWithInput:
            if case .script(let script) = result {
                let alert = NSAlert(); alert.messageText = script.title; alert.informativeText = L("Additional literal arguments, one per line")
                let field = NSTextView(frame: NSRect(x: 0, y: 0, width: 360, height: 100)); field.setAccessibilityLabel(L("Script arguments, one per line"))
                alert.accessoryView = field; alert.addButton(withTitle: L("Run")); alert.addButton(withTitle: L("Cancel"))
                isConfirmingClear = true; let response = alert.runModal(); isConfirmingClear = false
                if response == .alertFirstButtonReturn { onScript?(script, field.string.isEmpty ? [] : field.string.components(separatedBy: "\n")) }
                return
            }
            guard case .appleShortcut(let shortcut) = result else { return }
            let alert = NSAlert(); alert.messageText = L("Run Shortcut With Input")
            alert.informativeText = shortcut.name
            let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 340, height: 28))
            field.placeholderString = L("Text input"); field.setAccessibilityLabel(L("Shortcut text input"))
            alert.accessoryView = field; alert.addButton(withTitle: L("Run")); alert.addButton(withTitle: L("Cancel"))
            isConfirmingClear = true
            let response = alert.runModal(); isConfirmingClear = false
            if response == .alertFirstButtonReturn {
                let context = beginExternalAction()
                appleShortcuts.run(id: shortcut.id, input: field.stringValue) { [weak self] error in
                    if let error { self?.showExternalActionError(error, context: context) }
                }
            }

        case .pin, .unpin:
            if case .clipboard(let entry) = result {
                if !clipboard.setPinned(action == .pin, for: entry.id) { message = L("Clipboard pin limit reached") }
                updateResults(preserveSelection: true)
            } else if let pin = pin(for: result) { personalization.togglePin(pin); updateResults(preserveSelection: true) }
        case .editQuicklink: if case .quicklink(let link) = result { onEditQuicklink?(link) }
        case .copyPath:
            if let path { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(path, forType: .string) }
        case .reveal:
            if let path, FileManager.default.fileExists(atPath: path) { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)]) }
            else { message = L("This item no longer exists."); render() }
        case .openWith:
            guard let path else { return }
            guard FileManager.default.fileExists(atPath: path) else { message = "This file no longer exists."; render(); return }
            let picker = NSOpenPanel(); picker.title = L("Choose an application"); picker.directoryURL = URL(fileURLWithPath: "/Applications"); picker.allowedContentTypes = [.application]
            isConfirmingClear = true
            let response = picker.runModal(); isConfirmingClear = false
            if response == .OK, let application = picker.url {
                NSWorkspace.shared.open([URL(fileURLWithPath: path)], withApplicationAt: application, configuration: .init()) { [weak self] _, error in
                    Task { @MainActor in if let error { self?.message = error.localizedDescription; self?.render() } }
                }
            }
        }
    }

    @discardableResult
    func handleNoteDeleteKey(_ event: NSEvent) -> Bool {
        guard mode == .notes, noteEditorSession == nil,
              LauncherKeyboard.isNoteDeleteShortcut(event),
              results.indices.contains(selection), case .note(let note) = results[selection] else { return false }
        performNoteAction(.deleteNote, id: note.id)
        return true
    }

    private func performNoteAction(_ action: ResultAction, id: String) {
        let wasEditing = noteEditorSession?.isEditing == true
        guard saveNotes(), let store = loadedNotes, let note = store.note(id: id) else { return }
        switch action {
        case .editNote:
            if let session = noteEditorSession { _ = session.toggleEditing() }
            else { showNote(id: id) }
        case .copyMarkdown:
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(note.contents, forType: .string)
        case .exportNote:
            let picker = NSSavePanel()
            picker.title = L("Export Markdown Note")
            picker.nameFieldStringValue = note.title.replacingOccurrences(of: "/", with: "-")
                .replacingOccurrences(of: ":", with: "-") + ".md"
            picker.allowedContentTypes = [.plainText]
            isConfirmingClear = true
            let response = picker.runModal()
            isConfirmingClear = false
            if response == .OK, let url = picker.url {
                do { try store.export(id: id, to: url) }
                catch { message = error.localizedDescription }
            }
        case .deleteNote:
            store.remove(id: id)
            if store.note(id: id) == nil {
                if noteEditorSession != nil { enter(.notes) }
            } else { message = store.error }
        default: break
        }
        if wasEditing, action != .editNote, let session = noteEditorSession, !session.isEditing {
            _ = session.toggleEditing()
        }
        if noteEditorSession == nil { updateResults(preserveSelection: true) }
        focusInput()
    }

    func showLocalAssistant(query: String = "") {
        guard saveNotes() else { return }
        if !panel.isVisible { show() }
        localAssistant.reset()
        enter(.localAssistant)
        input.stringValue = query
        if !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            submitLocalAssistantPrompt()
        }
        focusInput()
    }

    func showChatHistory() {
        guard saveNotes() else { return }
        if !panel.isVisible { show() }
        localAssistant.cancel()
        enter(.chatHistory)
        focusInput()
    }

    private var matchingConversations: [LocalAssistantConversation] {
        LocalAssistantHistoryView.matches(localAssistant.history.conversations, query: input.stringValue)
    }

    func resumeLocalAssistantConversation(id: UUID) {
        guard saveNotes(), localAssistant.resumeConversation(id: id) else { return }
        if !panel.isVisible { show() }
        enter(.localAssistant)
        focusInput()
    }

    func removeLocalAssistantConversation(id: UUID) {
        localAssistant.removeConversation(id: id)
        if mode == .chatHistory {
            selection = min(selection, max(0, matchingConversations.count - 1))
            renderUtility()
            focusInput()
        }
    }

    private func submitLocalAssistantPrompt() {
        guard !localAssistant.isResponding else { return }
        let prompt = input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !prompt.isEmpty else { return }
        localAssistant.send(prompt)
        if localAssistant.isResponding { input.stringValue = "" }
    }

    @objc private func askAI() {
        guard mode == .root else { return }
        if ai.provider == .local { showLocalAssistant(query: input.stringValue) }
        else { onAI?(input.stringValue) }
    }
    func showHandoffError(query: String, error: String) {
        show()
        input.stringValue = query
        updateResults()
        message = error
        render()
        panel.makeFirstResponder(input)
    }

    private func isClipboardResult(_ result: LauncherResult) -> Bool {
        if case .clipboard = result { return true }; return false
    }
    private func removeSelectedClipboardItem() {
        guard results.indices.contains(selection), case .clipboard(let entry) = results[selection] else { return }
        clipboard.remove(id: entry.id)
    }
    @objc private func confirmClearHistory() {
        guard !clipboard.items.isEmpty else { return }
        isConfirmingClear = true
        let alert = NSAlert()
        alert.messageText = L("Clear clipboard history?")
        alert.informativeText = L("All saved clipboard items, including pinned items and attachments, will be removed.")
        alert.addButton(withTitle: L("Clear History"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons[0].hasDestructiveAction = true
        if alert.runModal() == .alertFirstButtonReturn { clipboard.clear() }
        isConfirmingClear = false
        panel.makeKey()
        panel.makeFirstResponder(input)
        updateResults()
    }

    private var emptyMessage: String {
        switch mode {
        case .notes: return L("No notes yet. Press ⌘N to create one.")
        case .dictionary, .dictation, .localAssistant, .chatHistory: return ""
        case .calculationHistory: return L("Copy a calculation to start your history")
        case .snippets: return L("Add snippets in Settings")
        case .root: return files.isSearching ? "Searching files…" : (index.isRefreshing ? "Finding your apps…" : "No results")
        case .clipboard:
            if !clipboard.items.isEmpty { return "No matching clipboard items" }
            return clipboard.enabled ? "Copy some text to start your history" : "Clipboard history is paused in Settings"
        case .webSearch: return quicklinks.webSearches.isEmpty ? "Add a web search template in Settings" : "Type a query, then press Return to search"
        case .quicklinks: return quicklinks.links.isEmpty ? "Add quicklinks in Settings" : "No quicklinks found"
        }
    }

    private func beginExternalAction() -> ExternalActionContext {
        let context = ExternalActionContext(query: input.stringValue, mode: mode,
                                            resultID: results[selection].id, revision: presentationRevision)
        dismiss(restoreFocus: false)
        return context
    }

    private func showExternalActionError(_ error: String, context: ExternalActionContext) {
        // A delayed completion must not replace a launcher session the user reopened.
        guard !panel.isVisible, presentationRevision == context.revision else { return }
        show()
        mode = context.mode
        setQuery(context.query)
        selection = results.firstIndex { $0.id == context.resultID } ?? 0
        message = error
        render()
        positionPanel()
        focusInput()
    }

    var isVisible: Bool { panel.isVisible }
    func performSelected() {
        if mode == .chatHistory {
            if matchingConversations.indices.contains(selection) {
                resumeLocalAssistantConversation(id: matchingConversations[selection].id)
            }
            return
        }
        if mode == .localAssistant { submitLocalAssistantPrompt(); return }
        if mode == .dictionary { lookupDictionary(); return }
        if mode == .dictation { toggleDictation(); return }
        guard panel.isVisible, results.indices.contains(selection) else { return }
        recordRecent(results[selection].id)
        switch results[selection] {
        case .appleShortcut(let shortcut):
            let context = beginExternalAction()
            appleShortcuts.run(id: shortcut.id, input: nil) { [weak self] error in
                if let error { self?.showExternalActionError(error, context: context) }
            }
        case .webSearch(let query):
            guard let url = quicklinks.fallbackURL(query: query) else { return }
            let context = beginExternalAction()
            if !NSWorkspace.shared.open(url) { showExternalActionError("Couldn’t open web search.", context: context) }
        case .command(let command):
            if let action = command.systemAction {
                let context = beginExternalAction()
                Task { @MainActor [weak self] in
                    do { try await SystemActionService.perform(action) }
                    catch { self?.showExternalActionError(error.localizedDescription, context: context) }
                }
                return
            }
            if let action = command.windowAction {
                let target = previousApp
                let context = beginExternalAction()
                do { try windowManagement.perform(action, application: target); target?.activate(options: []) }
                catch { showExternalActionError(error.localizedDescription, context: context) }
                return
            }
            switch command {
            case .calculationHistory: enter(.calculationHistory)
            case .snippets: enter(.snippets)
            case .notes:
                let search = LauncherUtilityQuery.noteSearch(in: input.stringValue)
                enter(.notes)
                if let search { setQuery(search) }
            case .dictionary:
                let word = LauncherUtilityQuery.dictionaryWord(in: input.stringValue)
                enter(.dictionary)
                if let word { setQuery(word); lookupDictionary() }
            case .chatHistory: showChatHistory()
            case .dictation: enter(.dictation)
            case .newNote: createNote()
            case .windowSearch, .menuSearch, .calendar, .joinNextMeeting, .selectedText, .raycastImport, .raycastExtensions, .windowLayouts: onCommand?(command)
            case .scripts, .extensions: onCommand?(command)
            case .clipboard: enter(.clipboard)
            case .quicklinks: enter(.quicklinks)
            case .emoji: onEmoji?()
            case .settings: onSettings?()
            case .webSearch: enter(.webSearch)
            case .browse, .help, .onboarding, .profiles, .appleShortcuts: onCommand?(command)
            case .lock, .sleep, .restart, .logOut, .toggleMute, .toggleAppearance, .leftHalf, .rightHalf, .maximizeWindow, .centerWindow, .restoreWindow, .nextDisplay, .topHalf, .bottomHalf, .topLeftQuarter, .topRightQuarter, .bottomLeftQuarter, .bottomRightQuarter, .leftThird, .centerThird, .rightThird, .leftTwoThirds, .rightTwoThirds, .previousDisplay: break
            case .shutdown:
                do {
                    try ShutdownService.requestConfirmation()
                    dismiss(restoreFocus: false)
                } catch {
                    message = "Couldn’t show shutdown confirmation: \(error.localizedDescription)"
                    render()
                }
            }
        case .quicklink(let link):
            let argument = quicklinks.resolve(query: input.stringValue).flatMap { $0.link.id == link.id ? $0.argument : nil } ?? ""
            guard let url = link.resolvedURL(argument: argument) else {
                message = "Couldn’t open this quicklink."; render(); return
            }
            let context = beginExternalAction()
            guard NSWorkspace.shared.open(url) else {
                showExternalActionError("Couldn’t open this quicklink.", context: context); return
            }
            usage[context.resultID, default: 0] += 1
            UserDefaults.standard.set(usage, forKey: "usage")
        case .note(let note): showNote(id: note.id)
        case .script(let script): onScript?(script, [])
        case .userExtension(let manifest): onExtension?(manifest)
        case .snippet(let snippet):
            isConfirmingClear = true
            let prepared = SnippetPresenter.prepare(snippet)
            isConfirmingClear = false
            if let prepared { if prepared.insert { pasteIntoPreviousApplication() } else { dismiss() } } else { focusInput() }
        case .clipboard(let entry):
            clipboard.copy(entry)
            pasteIntoPreviousApplication()
        case .file(let file):
            let context = beginExternalAction()
            if !NSWorkspace.shared.open(file.url) { showExternalActionError("Couldn’t open this file.", context: context) }
        case .historicalCalculation(let entry):
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(entry.result.clipboard, forType: .string)
            dismiss()
        case .calculation(let result):
            calculationHistory.recordAccepted(result)
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(result.clipboard, forType: .string)
            dismiss()
        case .application(let app):
            let context = beginExternalAction()
            launchApplication(app) { [weak self] error in
                guard let self else { return }
                if let error { self.showExternalActionError("Couldn’t open \(app.name): \(error.localizedDescription)", context: context) }
                else {
                    self.usage[app.id, default: 0] += 1
                    UserDefaults.standard.set(self.usage, forKey: "usage")
                }
            }
        }
    }
}
