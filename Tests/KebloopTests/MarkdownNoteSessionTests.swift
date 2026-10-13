import Foundation
import XCTest
@testable import Kebloop

@MainActor
final class MarkdownNoteSessionTests: XCTestCase {
    func testEscapeSavesDraftAndReturnsToPreview() throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = MarkdownNoteStore(directory: directory)
        let note = try XCTUnwrap(store.create(contents: "# Plan"))
        var didClose = false
        let session = MarkdownNoteSession(store: store, onClose: { didClose = true }, onUnsavedDraftChanged: { _ in })
        session.open(initialNoteID: note.id, initiallyEditing: true)
        session.setDraft("# Updated plan\n\nSaved by Escape.")

        XCTAssertTrue(session.escape())
        XCTAssertFalse(session.isEditing)
        XCTAssertFalse(didClose)
        XCTAssertEqual(store.note(id: note.id)?.contents, "# Updated plan\n\nSaved by Escape.")
        XCTAssertFalse(store.recoveryCandidates.contains(note.id))

        let reloaded = MarkdownNoteStore(directory: directory)
        XCTAssertEqual(reloaded.note(id: note.id)?.contents, "# Updated plan\n\nSaved by Escape.")
    }

    func testRejectedDraftBlocksEscapeCloseBrowseAndNewNoteWithoutLosingDraft() throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = MarkdownNoteStore(directory: directory)
        let note = try XCTUnwrap(store.create(contents: "Keep this note"))
        let otherNote = try XCTUnwrap(store.create(contents: "Another note"))
        var didClose = false
        var didBrowse = false
        let session = MarkdownNoteSession(store: store, onClose: { didClose = true }, onUnsavedDraftChanged: { _ in })
        session.open(initialNoteID: note.id, initiallyEditing: true)
        let rejectedDraft = String(repeating: "x", count: 2 * 1024 * 1024 + 1)
        session.setDraft(rejectedDraft)

        XCTAssertFalse(session.escape())
        XCTAssertFalse(session.close())
        XCTAssertFalse(session.saveAndBrowse { didBrowse = true })
        XCTAssertFalse(session.createBlankNote())
        session.open(initialNoteID: otherNote.id, initiallyEditing: true)
        XCTAssertTrue(session.isEditing)
        XCTAssertEqual(session.draft, rejectedDraft)
        XCTAssertEqual(session.selectedNoteID, note.id)
        XCTAssertEqual(store.note(id: note.id)?.contents, "Keep this note")
        XCTAssertFalse(didClose)
        XCTAssertFalse(didBrowse)
    }

    func testNewNoteIsBlankAndImmediatelyEntersFocusedEditorState() throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = MarkdownNoteStore(directory: directory)
        let session = MarkdownNoteSession(store: store, onClose: {}, onUnsavedDraftChanged: { _ in })

        XCTAssertTrue(session.createBlankNote())
        let note = try XCTUnwrap(session.selectedNote)
        XCTAssertEqual(note.contents, "")
        XCTAssertEqual(session.draft, "")
        XCTAssertTrue(session.isEditing)
        XCTAssertGreaterThan(session.editorFocusRevision, 0)
        XCTAssertEqual(try String(contentsOf: directory.appendingPathComponent(note.id).appendingPathExtension("md"), encoding: .utf8), "")
    }

    func testFindFromPreviewRequestsFindAndEntersEditor() throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = MarkdownNoteStore(directory: directory)
        let note = try XCTUnwrap(store.create(contents: "Find this phrase"))
        let session = MarkdownNoteSession(store: store, onClose: {}, onUnsavedDraftChanged: { _ in })
        session.open(initialNoteID: note.id, initiallyEditing: false)

        XCTAssertTrue(session.requestFind(.show))
        XCTAssertTrue(session.isEditing)
        XCTAssertEqual(session.findRequest, MarkdownNoteFindRequest(revision: 1, action: .show))
    }

    private func makeTemporaryDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("KebloopNoteSessionTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }
}
