import Foundation
import Carbon
import Combine

// Raw values are persisted identifiers; keep them stable across releases.
enum LauncherCommand: String, CaseIterable, Identifiable {
    case newNote, windowSearch, menuSearch, calendar, joinNextMeeting, selectedText, notes, dictionary, dictation, raycastImport, raycastExtensions, windowLayouts
    case lock, sleep, restart, logOut, toggleMute, toggleAppearance, leftHalf, rightHalf, maximizeWindow, centerWindow, restoreWindow, nextDisplay, calculationHistory
    case topHalf, bottomHalf, topLeftQuarter, topRightQuarter, bottomLeftQuarter, bottomRightQuarter, leftThird, centerThird, rightThird, leftTwoThirds, rightTwoThirds, previousDisplay
    case snippets, scripts, extensions, clipboard, quicklinks, emoji, settings, shutdown, browse, help, onboarding, profiles, webSearch, appleShortcuts
    var id: String { rawValue }
    var title: String {
        switch self {
        case .newNote: L("New Note")
        case .windowSearch: L("Search Windows")
        case .menuSearch: L("Search Menu Commands")
        case .calendar: L("Calendar and Meetings")
        case .joinNextMeeting: L("Join Next Meeting")
        case .selectedText: L("Selected Text Actions")
        case .notes: L("Floating Markdown Notes")
        case .dictionary: L("Dictionary Lookup")
        case .dictation: L("Dictation")
        case .raycastImport: L("Import Raycast Export")
        case .raycastExtensions: L("Raycast Compatible Commands")
        case .windowLayouts: L("Saved Window Layouts")
        case .lock: L("Lock Screen")
        case .sleep: L("Sleep")
        case .restart: L("Restart…")
        case .logOut: L("Log Out…")
        case .toggleMute: L("Toggle Mute")
        case .toggleAppearance: L("Toggle Appearance")
        case .leftHalf: L("Left Half")
        case .rightHalf: L("Right Half")
        case .maximizeWindow: L("Maximize Window")
        case .centerWindow: L("Center Window")
        case .restoreWindow: L("Restore Window")
        case .nextDisplay: L("Move to Next Display")
        case .topHalf: L("Top Half")
        case .bottomHalf: L("Bottom Half")
        case .topLeftQuarter: L("Top Left Quarter")
        case .topRightQuarter: L("Top Right Quarter")
        case .bottomLeftQuarter: L("Bottom Left Quarter")
        case .bottomRightQuarter: L("Bottom Right Quarter")
        case .leftThird: L("Left Third")
        case .centerThird: L("Center Third")
        case .rightThird: L("Right Third")
        case .leftTwoThirds: L("Left Two Thirds")
        case .rightTwoThirds: L("Right Two Thirds")
        case .previousDisplay: L("Move to Previous Display")
        case .calculationHistory: L("Calculator History")

        case .snippets: L("Snippets")
        case .scripts: L("Script Commands")
        case .extensions: L("Extensions")
        case .clipboard: L("Clipboard History")
        case .quicklinks: L("Quicklinks")
        case .emoji: L("Emoji & Symbols")
        case .settings: L("Open Settings")
        case .shutdown: L("Shut Down…")
        case .browse: L("Browse Commands")
        case .help: L("Help")
        case .onboarding: L("Welcome to Kebloop")
        case .profiles: L("Profiles")
        case .webSearch: L("Web Search")
        case .appleShortcuts: L("Apple Shortcuts")
        }
    }
    var symbol: String {
        switch self {
        case .newNote: "square.and.pencil"
        case .windowSearch: "macwindow"
        case .menuSearch: "menubar.rectangle"
        case .calendar: "calendar"
        case .joinNextMeeting: "video"
        case .selectedText: "text.cursor"
        case .notes: "note.text"
        case .dictionary: "character.book.closed"
        case .dictation: "mic"
        case .raycastImport: "square.and.arrow.down"
        case .raycastExtensions: "puzzlepiece.extension"
        case .windowLayouts: "rectangle.3.group"
        case .lock: "lock"
        case .sleep: "moon.zzz"
        case .restart: "restart"
        case .logOut: "rectangle.portrait.and.arrow.right"
        case .toggleMute: "speaker.slash"
        case .toggleAppearance: "circle.lefthalf.filled"
        case .leftHalf: "rectangle.lefthalf.filled"
        case .rightHalf: "rectangle.righthalf.filled"
        case .maximizeWindow: "arrow.up.left.and.arrow.down.right"
        case .centerWindow: "rectangle.center.inset.filled"
        case .restoreWindow: "arrow.uturn.backward"
        case .nextDisplay: "display.2"
        case .topHalf: "rectangle.tophalf.filled"
        case .bottomHalf: "rectangle.bottomhalf.filled"
        case .topLeftQuarter: "rectangle.split.2x2"
        case .topRightQuarter: "rectangle.split.2x2"
        case .bottomLeftQuarter: "rectangle.split.2x2"
        case .bottomRightQuarter: "rectangle.split.2x2"
        case .leftThird: "rectangle.split.3x1"
        case .centerThird: "rectangle.split.3x1"
        case .rightThird: "rectangle.split.3x1"
        case .leftTwoThirds: "rectangle.split.3x1"
        case .rightTwoThirds: "rectangle.split.3x1"
        case .previousDisplay: "display.2"
        case .calculationHistory: "clock.arrow.circlepath"

        case .snippets: "text.quote"
        case .scripts: "terminal"
        case .extensions: "puzzlepiece.extension"
        case .clipboard: "doc.on.clipboard"
        case .quicklinks: "link"
        case .emoji: "face.smiling"
        case .settings: "gearshape"
        case .shutdown: "power"
        case .browse: "square.grid.2x2"
        case .help: "questionmark.circle"
        case .onboarding: "sparkles"
        case .profiles: "person.crop.circle"
        case .webSearch: "globe"
        case .appleShortcuts: "square.stack.3d.up"
        }
    }
    var aliases: [String] {
        switch self {
        case .newNote: ["new note", "quick note", "nowa notatka"]
        case .windowSearch: ["windows", "window search"]
        case .menuSearch: ["menu search", "menu commands"]
        case .calendar: ["calendar", "meetings"]
        case .joinNextMeeting: ["join next meeting", "join meeting"]
        case .selectedText: ["selected text", "rewrite", "translate", "summarize", "grammar"]
        case .notes: ["notes", "markdown", "floating notes"]
        case .dictionary: ["dictionary", "define", "lookup"]
        case .dictation: ["dictation", "dictate"]
        case .raycastImport: ["raycast import", "raycast migration"]
        case .raycastExtensions: ["raycast extensions", "raycast commands"]
        case .windowLayouts: ["window layouts", "save layout", "restore layout"]
        case .lock: ["lock screen", "lock", "zablokuj"]
        case .sleep: ["sleep", "uśpij"]
        case .restart: ["restart", "reboot", "uruchom ponownie"]
        case .logOut: ["log out", "logout", "wyloguj"]
        case .toggleMute: ["mute", "unmute", "wycisz"]
        case .toggleAppearance: ["dark mode", "light mode", "appearance", "wygląd"]
        case .leftHalf: ["left half", "left window", "lewa połowa"]
        case .rightHalf: ["right half", "right window", "prawa połowa"]
        case .maximizeWindow: ["maximize", "window maximize", "maksymalizuj"]
        case .centerWindow: ["center window", "center", "wyśrodkuj"]
        case .restoreWindow: ["restore window", "restore", "przywróć okno"]
        case .nextDisplay: ["next display", "move display", "następny ekran"]
        case .topHalf: ["top half", "window top half"]
        case .bottomHalf: ["bottom half", "window bottom half"]
        case .topLeftQuarter: ["top left quarter", "window top left quarter"]
        case .topRightQuarter: ["top right quarter", "window top right quarter"]
        case .bottomLeftQuarter: ["bottom left quarter", "window bottom left quarter"]
        case .bottomRightQuarter: ["bottom right quarter", "window bottom right quarter"]
        case .leftThird: ["left third", "window left third"]
        case .centerThird: ["center third", "window center third"]
        case .rightThird: ["right third", "window right third"]
        case .leftTwoThirds: ["left two thirds", "window left two thirds"]
        case .rightTwoThirds: ["right two thirds", "window right two thirds"]
        case .previousDisplay: ["move to previous display", "window move to previous display"]
        case .calculationHistory: ["calculator history", "calculation history", "historia obliczeń"]

        case .snippets: ["snippets", "text", "fragmenty", "tekst"]
        case .scripts: ["scripts", "script commands", "skrypty"]
        case .extensions: ["extensions", "rozszerzenia"]
        case .clipboard: ["schowek", "historia", "clipboard", "history", "paste"]
        case .quicklinks: ["linki", "zakładki", "quicklinks", "links", "bookmarks"]
        case .emoji: ["emotikony", "symbole", "emoji", "symbols", "emoji & symbols"]
        case .settings: ["ustawienia", "preferencje", "settings", "preferences", "open settings"]
        case .shutdown: ["wyłącz", "zamknij komputer", "shutdown", "shut down", "power off", "turn off"]
        case .browse: ["polecenia", "przeglądaj", "browse", "commands", "all commands"]
        case .help: ["pomoc", "poradnik", "help", "guide"]
        case .onboarding: ["witaj", "samouczek", "welcome", "onboarding", "tutorial"]
        case .profiles: ["profile", "przełącz profil", "profiles", "switch profile"]
        case .webSearch: ["szukaj w sieci", "wyszukiwarka", "web search", "search web"]
        case .appleShortcuts: ["skróty", "automatyzacja", "apple shortcuts", "shortcuts", "automation"]
        }
    }
    var systemAction: SystemActionService.Action? {
        switch self {
        case .lock: .lock
        case .sleep: .sleep
        case .restart: .restart
        case .logOut: .logOut
        case .toggleMute: .toggleMute
        case .toggleAppearance: .toggleAppearance
        default: nil
        }
    }
    var windowAction: WindowManagementService.Action? {
        switch self {
        case .leftHalf: .leftHalf
        case .rightHalf: .rightHalf
        case .maximizeWindow: .maximize
        case .centerWindow: .center
        case .restoreWindow: .restore
        case .nextDisplay: .nextDisplay
        case .topHalf: .topHalf
        case .bottomHalf: .bottomHalf
        case .topLeftQuarter: .topLeftQuarter
        case .topRightQuarter: .topRightQuarter
        case .bottomLeftQuarter: .bottomLeftQuarter
        case .bottomRightQuarter: .bottomRightQuarter
        case .leftThird: .leftThird
        case .centerThird: .centerThird
        case .rightThird: .rightThird
        case .leftTwoThirds: .leftTwoThirds
        case .rightTwoThirds: .rightTwoThirds
        case .previousDisplay: .previousDisplay
        default: nil
        }
    }

}

struct CommandShortcutBinding: Codable, Identifiable {
    let id: String
    let keyCode: UInt32
    let modifiers: UInt32
    let label: String
}

@MainActor
final class CommandShortcutStore: ObservableObject {
    @Published private(set) var bindings: [CommandShortcutBinding] = []
    @Published private(set) var errors: [String: String] = [:]
    @Published var recorderConflict: String?
    var isEnabled: (String) -> Bool = { _ in true }
    var onCommand: ((String) -> Void)?
    private let hotkey: HotkeyService
    private let defaults: UserDefaults
    init(hotkey: HotkeyService, defaults: UserDefaults = .standard) {
        self.hotkey = hotkey
        self.defaults = defaults
        reload()
    }
    func unregisterAll() { for binding in bindings { hotkey.unregisterBinding(id: "command:\(binding.id)") } }
    func reload(register: Bool = true) {
        unregisterAll()
        bindings = defaults.data(forKey: "commandShortcuts").flatMap { try? JSONDecoder().decode([CommandShortcutBinding].self, from: $0) } ?? []
        if register { resume() }
    }
    func suspend() { hotkey.beginRecording() }
    func resume() {
        unregisterAll()
        errors = [:]
        for binding in bindings where isEnabled(binding.id) { register(binding) }
        hotkey.endRecording()
        for binding in bindings where hotkey.registrationErrors.contains("command:\(binding.id)") { errors[binding.id] = "Shortcut unavailable. It may be used by macOS or another app." }
    }
    @discardableResult
    func assign(commandID: String, keyCode: UInt32, modifiers: UInt32, label: String) -> Bool {
        guard modifiers & UInt32(cmdKey | optionKey | controlKey) != 0 else { recorderConflict = "Include Command, Option, or Control."; return false }
        guard !hotkey.conflictsWithLauncher(keyCode: keyCode, modifiers: modifiers) else { recorderConflict = "This shortcut opens Kebloop."; return false }
        guard !hotkey.conflictsWithBinding(id: "command:\(commandID)", keyCode: keyCode, modifiers: modifiers) else { recorderConflict = "This shortcut is already assigned to an app or command."; return false }
        let binding = CommandShortcutBinding(id: commandID, keyCode: keyCode, modifiers: modifiers, label: label)
        guard register(binding) else { recorderConflict = "Shortcut unavailable. It may be used by macOS or another app."; return false }
        if let offset = bindings.firstIndex(where: { $0.id == commandID }) { bindings[offset] = binding } else { bindings.append(binding) }
        persist()
        recorderConflict = nil
        return true
    }
    func remove(id: String) {
        hotkey.unregisterBinding(id: "command:\(id)")
        bindings.removeAll { $0.id == id }
        errors[id] = nil
        persist()
    }
    @discardableResult
    private func register(_ binding: CommandShortcutBinding) -> Bool {
        guard isEnabled(binding.id) else { hotkey.unregisterBinding(id: "command:\(binding.id)"); errors[binding.id] = nil; return true }
        let success = hotkey.registerBinding(id: "command:\(binding.id)", keyCode: binding.keyCode, modifiers: binding.modifiers) { [weak self] in self?.onCommand?(binding.id) }
        errors[binding.id] = success ? nil : "Shortcut unavailable or already assigned."
        return success
    }
    private func persist() {
        if let data = try? JSONEncoder().encode(bindings) { defaults.set(data, forKey: "commandShortcuts") }
    }
}
