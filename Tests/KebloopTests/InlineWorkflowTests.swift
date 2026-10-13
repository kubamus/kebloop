import AppKit
import XCTest
@testable import Kebloop

final class LauncherUtilityQueryTests: XCTestCase {
    func testUtilityKeywordsPreserveUnicodeAndOnlyConsumeWholePrefixes() {
        XCTAssertEqual(LauncherUtilityQuery.dictionaryWord(in: " DEFINE déjà vu "), "déjà vu")
        XCTAssertEqual(LauncherUtilityQuery.noteSearch(in: "notes  Meeting plan"), "Meeting plan")
        XCTAssertNil(LauncherUtilityQuery.dictionaryWord(in: "redefine word"))
        XCTAssertNil(LauncherUtilityQuery.dictionaryWord(in: "define"))
        XCTAssertNil(LauncherUtilityQuery.noteSearch(in: "notes"))
    }
}

@MainActor
final class InlineWorkflowTests: XCTestCase {
    func testNotesBrowseAndDirectQueryOpenExistingNoteWithoutCreatingAnother() async throws {
        try await withLauncher { launcher, notes in
            let note = try XCTUnwrap(notes.create(contents: "# Keyboard plan\n\nUseful notes"))
            launcher.show()
            launcher.setQuery("notes Keyboard plan")
            launcher.performSelected()
            XCTAssertEqual(launcher.currentModeID, "notes")
            XCTAssertEqual(launcher.resultIdentifiers.first, "note:" + note.id)
            launcher.performSelected()
            XCTAssertEqual(launcher.currentEditingNoteID, note.id)
            XCTAssertTrue(launcher.isVisible)
            XCTAssertEqual(notes.notes.count, 1)
        }
    }

    func testDictionaryUsesLauncherAndIgnoresResultsForAnEditedQuery() async throws {
        try await withLauncher { launcher, _ in
            launcher.show()
            XCTAssertTrue(launcher.resultIdentifiers.contains("command:dictionary"))
            XCTAssertTrue(launcher.resultIdentifiers.contains("command:dictation"))
            launcher.setQuery("define first")
            launcher.performSelected()
            XCTAssertEqual(launcher.currentModeID, "dictionary")
            XCTAssertEqual(launcher.currentQuery, "first")
            launcher.setQuery("second")
            launcher.lookupDictionary()
            for _ in 0..<50 {
                if launcher.currentDictionaryDefinition == "Definition: second" { break }
                try await Task.sleep(for: .milliseconds(10))
            }
            try await Task.sleep(for: .milliseconds(120))
            XCTAssertEqual(launcher.currentDictionaryDefinition, "Definition: second")
            XCTAssertTrue(launcher.isVisible)
        }
    }

    func testEscapeReturnsInlineToolsToTheLauncher() async throws {
        try await withLauncher { launcher, _ in
            let escape = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
                timestamp: 0, windowNumber: 0, context: nil, characters: "\u{1b}",
                charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
            for command in [LauncherCommand.dictionary, .dictation] {
                launcher.executeCommand(id: command.id)
                XCTAssertTrue(launcher.handleUtilityKey(escape))
                XCTAssertEqual(launcher.currentModeID, "root")
                XCTAssertTrue(launcher.isVisible)
            }
        }
    }

    private func withLauncher(_ body: (LauncherPanelController, MarkdownNoteStore) async throws -> Void) async throws {
        _ = NSApplication.shared
        let suite = "InlineWorkflow.\(UUID())"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defaults.set(false, forKey: "clipboardHistoryEnabled")
        let pasteboard = NSPasteboard(name: .init(suite))
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(suite)
        defer {
            defaults.removePersistentDomain(forName: suite)
            pasteboard.releaseGlobally()
            try? FileManager.default.removeItem(at: directory)
        }
        let notes = MarkdownNoteStore(directory: directory.appendingPathComponent("Notes"))
        let launcher = LauncherPanelController(index: ApplicationIndex(), quicklinks: QuicklinkStore(defaults: defaults),
            clipboard: ClipboardHistoryStore(defaults: defaults, pasteboard: pasteboard, storageURL: directory.appendingPathComponent("history.json")),
            aliases: AppAliasStore(defaults: defaults), ai: AIHandoffService(defaults: defaults, pasteboard: pasteboard),
            personalization: PersonalizationStore(defaults: defaults), appleShortcuts: AppleShortcutsService(defaults: defaults),
            dictationService: OnDeviceDictationService(pasteboard: pasteboard, onDeviceSupportOverride: false),
            lookupDefinition: { word in
                if word == "first" { Thread.sleep(forTimeInterval: 0.1) }
                return "Definition: " + word
            })
        launcher.noteStore = { notes }
        defer { launcher.dismiss(restoreFocus: false); notes.flushAll() }
        try await body(launcher, notes)
    }
}
