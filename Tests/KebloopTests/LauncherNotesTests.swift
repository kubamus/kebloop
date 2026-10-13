import AppKit
import XCTest
@testable import Kebloop

@MainActor
final class LauncherNotesTests: XCTestCase {
    func testNotesCommandPreviewsSelectedNoteAndTracksSearchSelection() async throws {
        try await withLauncher { launcher, notes in
            _ = try XCTUnwrap(notes.create(contents: "# First note\n\nFirst body"))
            _ = try XCTUnwrap(notes.create(contents: "# Second note\n\nSecond body"))

            launcher.executeCommand(id: LauncherCommand.notes.id)

            XCTAssertEqual(launcher.currentModeID, "notes")
            XCTAssertEqual(launcher.resultIdentifiers.count, 3)
            let firstVisibleNoteID = try XCTUnwrap(launcher.resultIdentifiers.first?.replacingOccurrences(of: "note:", with: ""))
            XCTAssertEqual(launcher.currentPreviewNoteID, firstVisibleNoteID)

            launcher.moveSelection(1)
            let secondVisibleNoteID = try XCTUnwrap(launcher.resultIdentifiers.dropFirst().first?.replacingOccurrences(of: "note:", with: ""))
            XCTAssertEqual(launcher.currentPreviewNoteID, secondVisibleNoteID)

            launcher.setQuery("First body")
            XCTAssertEqual(launcher.currentPreviewNoteID, notes.notes.first(where: { $0.contents.contains("First body") })?.id)
        }
    }

    func testCreatingNoteStartsBlankEmbeddedEditorAndTracksItsID() async throws {
        try await withLauncher { launcher, notes in
            launcher.executeCommand(id: LauncherCommand.notes.id)
            launcher.createNote()

            let noteID = try XCTUnwrap(launcher.currentEditingNoteID)
            XCTAssertEqual(launcher.currentModeID, "notes")
            XCTAssertTrue(launcher.isVisible)
            XCTAssertEqual(notes.note(id: noteID)?.contents, "")
            XCTAssertEqual(launcher.currentPreviewNoteID, noteID)
        }
    }

    func testExistingNoteOpensInTheLauncherPanelForPreviewAndEditing() async throws {
        try await withLauncher { launcher, notes in
            let note = try XCTUnwrap(notes.create(contents: "# Existing note\n\nKeep this in the launcher"))

            launcher.executeCommand(id: LauncherCommand.notes.id)
            launcher.showNote(id: note.id, editing: false)

            XCTAssertTrue(launcher.isVisible)
            XCTAssertEqual(launcher.currentPreviewNoteID, note.id)
            XCTAssertNil(launcher.currentEditingNoteID)

            launcher.showNote(id: note.id, editing: true)

            XCTAssertTrue(launcher.isVisible)
            XCTAssertEqual(launcher.currentEditingNoteID, note.id)
            XCTAssertEqual(notes.notes.count, 1)
        }
    }

    func testCommandKOffersActionsForNotePreviewAndEditor() async throws {
        try await withLauncher { launcher, notes in
            let note = try XCTUnwrap(notes.create(contents: "# Action target\n\nNote actions"))
            let expectedActions: [ResultAction] = [.editNote, .copyMarkdown, .exportNote, .deleteNote]

            launcher.executeCommand(id: LauncherCommand.notes.id)
            launcher.setQuery("Action target")
            launcher.showActions()
            XCTAssertEqual(launcher.visibleActionIdentifiers, expectedActions)

            let escape = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
                timestamp: 0, windowNumber: 0, context: nil, characters: "\u{1b}",
                charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
            XCTAssertTrue(launcher.handleActionKey(escape))

            launcher.showNote(id: note.id, editing: true)
            let window = try XCTUnwrap(launcherWindow())
            try await waitForLayout(in: window) {
                descendants(of: window.contentView).contains {
                    ($0 as? MarkdownNoteTextView)?.accessibilityLabel() == L("Markdown note editor")
                }
            }
            launcher.showActions()
            XCTAssertEqual(launcher.visibleActionIdentifiers, expectedActions)
            XCTAssertTrue(launcher.handleActionKey(escape))
            try await waitForLayout(in: window) { window.firstResponder is MarkdownNoteTextView }
            XCTAssertTrue(window.firstResponder is MarkdownNoteTextView,
                          "Closing the actions menu should return focus to the embedded note editor")
        }
    }

    func testCommandBackspaceDeletesSelectedNoteAndPreservesSearch() async throws {
        try await withLauncher { launcher, notes in
            let selected = try XCTUnwrap(notes.create(contents: "# Delete target"))
            let remaining = try XCTUnwrap(notes.create(contents: "# Keep this note"))
            launcher.executeCommand(id: LauncherCommand.notes.id)
            launcher.setQuery("Delete target")

            XCTAssertFalse(launcher.handleNoteDeleteKey(try keyEvent(code: 51, modifiers: [])))
            XCTAssertFalse(launcher.handleNoteDeleteKey(try keyEvent(code: 51, modifiers: [.command, .shift])))
            XCTAssertNotNil(notes.note(id: selected.id))
            XCTAssertTrue(launcher.handleNoteDeleteKey(try keyEvent(code: 51, modifiers: .command)))

            XCTAssertNil(notes.note(id: selected.id))
            XCTAssertNotNil(notes.note(id: remaining.id))
            XCTAssertEqual(launcher.currentQuery, "Delete target")
            XCTAssertTrue(launcher.isVisible)
            XCTAssertFalse(FileManager.default.fileExists(atPath:
                notes.selectedNotesDirectory.appendingPathComponent(selected.id + ".md").path))
        }
    }

    func testDeleteMenuActionRemovesNoteImmediately() async throws {
        try await withLauncher { launcher, notes in
            let note = try XCTUnwrap(notes.create(contents: "# Delete from menu"))
            launcher.executeCommand(id: LauncherCommand.notes.id)
            launcher.showActions()
            for _ in 0..<3 { XCTAssertTrue(launcher.handleActionKey(try keyEvent(code: 125))) }
            XCTAssertTrue(launcher.handleActionKey(try keyEvent(code: 36)))

            XCTAssertNil(notes.note(id: note.id))
            XCTAssertFalse(launcher.isShowingActions)
            XCTAssertTrue(launcher.isVisible)
        }
    }

    func testCommandBackspaceAlsoWorksWhileNoteActionsAreOpen() async throws {
        try await withLauncher { launcher, notes in
            let note = try XCTUnwrap(notes.create(contents: "# Delete with menu shortcut"))
            launcher.executeCommand(id: LauncherCommand.notes.id)
            launcher.showActions()
            XCTAssertTrue(launcher.handleActionKey(try keyEvent(code: 51, modifiers: .command)))
            XCTAssertNil(notes.note(id: note.id))
            XCTAssertFalse(launcher.isShowingActions)
        }
    }

    private func keyEvent(code: UInt16, modifiers: NSEvent.ModifierFlags = []) throws -> NSEvent {
        try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: modifiers,
            timestamp: 0, windowNumber: 0, context: nil, characters: "", charactersIgnoringModifiers: "",
            isARepeat: false, keyCode: code))
    }

    func testNotesPanelKeepsListAndPreviewTogetherAndScrollUpdatesThePreview() async throws {
        try await withLauncher { launcher, notes in
            for index in 0..<15 {
                _ = try XCTUnwrap(notes.create(contents: "# Note \(index)\n\nBody for note \(index)"))
            }
            launcher.executeCommand(id: LauncherCommand.notes.id)

            let window = try XCTUnwrap(launcherWindow())
            try await waitForLayout(in: window) {
                descendants(of: window.contentView).contains {
                    guard let scrollView = $0 as? NSScrollView else { return false }
                    return isVisibleInHierarchy(scrollView) && scrollView.documentView is NSStackView
                }
            }

            let content = try XCTUnwrap(window.contentView)
            content.layoutSubtreeIfNeeded()
            let list = try XCTUnwrap(descendants(of: content).compactMap { $0 as? NSScrollView }.first {
                isVisibleInHierarchy($0) && $0.documentView is NSStackView
            })
            let rows = try XCTUnwrap(list.documentView as? NSStackView)
            let body = try XCTUnwrap(ancestorStack(containing: list))
            let preview = try XCTUnwrap(body.arrangedSubviews.first { $0 !== list && !$0.isHidden })

            XCTAssertGreaterThan(list.frame.width, 0)
            XCTAssertGreaterThan(preview.frame.width, 0)
            let listFrame = list.convert(list.bounds, to: body)
            let previewFrame = preview.convert(preview.bounds, to: body)
            XCTAssertGreaterThanOrEqual(listFrame.minX, body.bounds.minX - 1)
            XCTAssertLessThanOrEqual(listFrame.maxX, body.bounds.maxX + 1)
            XCTAssertGreaterThanOrEqual(previewFrame.minX, body.bounds.minX - 1)
            XCTAssertLessThanOrEqual(previewFrame.maxX, body.bounds.maxX + 1)

            let initialPreviewID = try XCTUnwrap(launcher.currentPreviewNoteID)
            let targetIndex = 10
            let targetRow = rows.arrangedSubviews[targetIndex]
            let clipView = list.contentView
            let maximumOffset = max(0, rows.frame.height - clipView.bounds.height)
            let targetOffset = min(maximumOffset, max(0, targetRow.frame.midY - clipView.bounds.height / 2))
            clipView.scroll(to: NSPoint(x: 0, y: targetOffset))
            list.reflectScrolledClipView(clipView)
            let visibleNoteIndex = try XCTUnwrap(rows.arrangedSubviews.indices.first { index in
                clipView.bounds.intersects(rows.arrangedSubviews[index].frame)
                    && launcher.resultIdentifiers[index].hasPrefix("note:")
            })
            let visibleNoteID = String(launcher.resultIdentifiers[visibleNoteIndex].dropFirst("note:".count))
            XCTAssertNotEqual(initialPreviewID, visibleNoteID)
            NotificationCenter.default.post(name: NSScrollView.didLiveScrollNotification, object: list)

            XCTAssertEqual(launcher.currentPreviewNoteID, visibleNoteID,
                           "Scrolling the notes list should preview the first visible note automatically")
        }
    }

    func testEmbeddedEditorAndPreviewBothFitInsideTheLauncherPanel() async throws {
        try await withLauncher { launcher, notes in
            let note = try XCTUnwrap(notes.create(contents: "# Editor width check\n\nPreview text stays beside the editor."))
            launcher.showNote(id: note.id, editing: true)

            let window = try XCTUnwrap(launcherWindow())
            try await waitForLayout(in: window) {
                descendants(of: window.contentView).contains {
                    ($0 as? NSTextView)?.accessibilityLabel() == L("Markdown note editor")
                }
            }

            let content = try XCTUnwrap(window.contentView)
            content.layoutSubtreeIfNeeded()
            let views = descendants(of: content)
            let editor = try XCTUnwrap(views.compactMap { $0 as? MarkdownNoteTextView }.first {
                isVisibleInHierarchy($0) && $0.accessibilityLabel() == L("Markdown note editor")
            })
            let scrollViews = views.compactMap { $0 as? NSScrollView }.filter { isVisibleInHierarchy($0) }
            let editorScroll = try XCTUnwrap(scrollViews.first { $0.documentView === editor })
            let previewScroll = try XCTUnwrap(scrollViews.first {
                $0 !== editorScroll
            })

            XCTAssertGreaterThan(editorScroll.frame.width, 0)
            XCTAssertGreaterThan(previewScroll.frame.width, 0)
            let editorFrame = editorScroll.convert(editorScroll.bounds, to: content)
            let previewFrame = previewScroll.convert(previewScroll.bounds, to: content)
            XCTAssertLessThanOrEqual(editorFrame.maxX, previewFrame.minX + 1,
                                     "The note preview should sit beside the editor without overlap")
            XCTAssertGreaterThanOrEqual(editorFrame.minX, content.bounds.minX - 1)
            XCTAssertLessThanOrEqual(editorFrame.maxX, content.bounds.maxX + 1)
            XCTAssertGreaterThanOrEqual(previewFrame.minX, content.bounds.minX - 1)
            XCTAssertLessThanOrEqual(previewFrame.maxX, content.bounds.maxX + 1)

            let searchField = try XCTUnwrap(views.compactMap { $0 as? NSTextField }.first {
                $0.accessibilityLabel() == L("Search")
            })
            XCTAssertEqual(editor.string, note.contents)
            XCTAssertNotEqual(editor.string, searchField.stringValue,
                              "Note contents belong to the embedded editor, not the launcher's search field")
            if let searchFieldEditor = searchField.currentEditor() as? NSTextView {
                XCTAssertFalse(editor === searchFieldEditor)
            }
        }
    }

    private func withLauncher(_ body: (LauncherPanelController, MarkdownNoteStore) async throws -> Void) async throws {
        _ = NSApplication.shared
        let suite = "LauncherNotesTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defaults.set(false, forKey: "clipboardHistoryEnabled")
        let pasteboard = NSPasteboard(name: .init(suite))
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(suite, isDirectory: true)
        defer {
            defaults.removePersistentDomain(forName: suite)
            pasteboard.releaseGlobally()
            try? FileManager.default.removeItem(at: directory)
        }

        let notes = MarkdownNoteStore(directory: directory.appendingPathComponent("Notes", isDirectory: true))
        let launcher = LauncherPanelController(index: ApplicationIndex(), quicklinks: QuicklinkStore(defaults: defaults),
            clipboard: ClipboardHistoryStore(defaults: defaults, pasteboard: pasteboard,
                storageURL: directory.appendingPathComponent("history.json")),
            aliases: AppAliasStore(defaults: defaults), ai: AIHandoffService(defaults: defaults, pasteboard: pasteboard),
            personalization: PersonalizationStore(defaults: defaults), appleShortcuts: AppleShortcutsService(defaults: defaults),
            dictationService: OnDeviceDictationService(pasteboard: pasteboard, onDeviceSupportOverride: false))
        launcher.noteStore = { notes }
        defer { launcher.dismiss(restoreFocus: false); notes.flushAll() }
        try await body(launcher, notes)
    }

    private func launcherWindow() -> NSWindow? {
        NSApp.windows.first { $0.isVisible && $0.accessibilityLabel() == L("Kebloop launcher") }
            ?? NSApp.windows.first { $0.isVisible && String(describing: type(of: $0)).contains("LauncherPanel") }
    }

    private func descendants(of view: NSView?) -> [NSView] {
        guard let view else { return [] }
        return [view] + view.subviews.flatMap { descendants(of: $0) }
    }

    private func isVisibleInHierarchy(_ view: NSView) -> Bool {
        var current: NSView? = view
        while let candidate = current {
            if candidate.isHidden { return false }
            current = candidate.superview
        }
        return true
    }

    private func ancestorStack(containing view: NSView) -> NSStackView? {
        var ancestor = view.superview
        while let current = ancestor {
            if let stack = current as? NSStackView, stack.arrangedSubviews.contains(where: { $0 === view }) {
                return stack
            }
            ancestor = current.superview
        }
        return nil
    }

    private func waitForLayout(in window: NSWindow, until predicate: () -> Bool) async throws {
        for _ in 0..<50 {
            window.contentView?.layoutSubtreeIfNeeded()
            if predicate() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Timed out waiting for the launcher panel's native note views")
    }
}
