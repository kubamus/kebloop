import AppKit
import XCTest
@testable import Kebloop

@MainActor
final class WorkflowCommandTests: XCTestCase {
    func testWorkflowCommandsAreSearchableAndRouteToTheirPresenter() throws {
        _ = NSApplication.shared
        let suite = "WorkflowCommands.\(UUID())"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set(false, forKey: "clipboardHistoryEnabled")
        let pasteboard = NSPasteboard(name: .init(suite))
        defer { pasteboard.releaseGlobally() }
        let historyURL = FileManager.default.temporaryDirectory.appendingPathComponent(suite + ".json")
        defer { try? FileManager.default.removeItem(at: historyURL) }
        let launcher = LauncherPanelController(index: ApplicationIndex(), quicklinks: QuicklinkStore(defaults: defaults),
            clipboard: ClipboardHistoryStore(defaults: defaults, pasteboard: pasteboard, storageURL: historyURL),
            aliases: AppAliasStore(defaults: defaults), ai: AIHandoffService(defaults: defaults, pasteboard: pasteboard),
            personalization: PersonalizationStore(defaults: defaults), appleShortcuts: AppleShortcutsService(defaults: defaults))
        let notesURL = FileManager.default.temporaryDirectory.appendingPathComponent(suite)
        let notes = MarkdownNoteStore(directory: notesURL)
        launcher.noteStore = { notes }
        defer { launcher.dismiss(restoreFocus: false); notes.flushAll(); try? FileManager.default.removeItem(at: notesURL) }
        let commands: [LauncherCommand] = [.windowSearch, .menuSearch, .calendar, .joinNextMeeting, .selectedText,
            .raycastImport, .raycastExtensions, .windowLayouts]
        launcher.show()
        launcher.setQuery("")
        XCTAssertTrue(launcher.resultIdentifiers.contains("command:browse"))
        XCTAssertLessThanOrEqual(launcher.resultIdentifiers.filter { $0.hasPrefix("command:") }.count, 12,
                       "The empty panel must not build every command row as the registry grows")
        var invoked: [LauncherCommand] = []
        launcher.onCommand = { invoked.append($0) }
        for command in commands {
            launcher.show()
            launcher.setQuery(command.aliases[0])
            XCTAssertTrue(launcher.resultIdentifiers.contains("command:" + command.id), command.id)
            launcher.executeCommand(id: command.id)
            XCTAssertEqual(invoked.last, command)
        }
        XCTAssertEqual(invoked, commands)
        for (command, mode) in [(LauncherCommand.dictionary, "dictionary"), (.dictation, "dictation"), (.newNote, "notes")] {
            launcher.executeCommand(id: command.id)
            XCTAssertEqual(launcher.currentModeID, mode)
            XCTAssertTrue(launcher.isVisible)
            XCTAssertEqual(invoked, commands, "Inline tools must not request separate presenter windows")
        }
    }
}
