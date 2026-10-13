import Foundation
import XCTest
@testable import Kebloop

final class CalendarAndNotesTests: XCTestCase {
    func testCalendarEligibilityExcludesCanceledAllDayFinishedAndDeclinedEvents() {
        let now = Date(timeIntervalSince1970: 10_000)

        XCTAssertTrue(CalendarMeetingService.isEligible(isCanceled: false, allDay: false,
                                                        endDate: now.addingTimeInterval(60),
                                                        currentUserDeclined: false, at: now))
        XCTAssertFalse(CalendarMeetingService.isEligible(isCanceled: true, allDay: false,
                                                         endDate: now.addingTimeInterval(60),
                                                         currentUserDeclined: false, at: now))
        XCTAssertFalse(CalendarMeetingService.isEligible(isCanceled: false, allDay: true,
                                                         endDate: now.addingTimeInterval(60),
                                                         currentUserDeclined: false, at: now))
        XCTAssertFalse(CalendarMeetingService.isEligible(isCanceled: false, allDay: false,
                                                         endDate: now, currentUserDeclined: false, at: now))
        XCTAssertFalse(CalendarMeetingService.isEligible(isCanceled: false, allDay: false,
                                                         endDate: now.addingTimeInterval(60),
                                                         currentUserDeclined: true, at: now))
    }

    func testMeetingLinkPrefersVideoProviderLinkAndRejectsCustomSchemes() throws {
        let unrelated = try XCTUnwrap(URL(string: "https://company.example/events/42"))

        let meeting = CalendarMeetingService.meetingURL(
            eventURL: unrelated,
            location: "Room 4",
            notes: "Agenda and join at https://zoom.us/j/123456789."
        )
        XCTAssertEqual(meeting?.absoluteString, "https://zoom.us/j/123456789")

        let unsafe = try XCTUnwrap(URL(string: "custom://zoom.us/j/123"))
        XCTAssertNil(CalendarMeetingService.meetingURL(eventURL: unsafe, location: nil, notes: nil))
        XCTAssertEqual(CalendarMeetingService.meetingURL(eventURL: unrelated, location: nil, notes: nil), unrelated)
    }
}

@MainActor
final class MarkdownNoteStoreTests: XCTestCase {
    func testImmediateEditsRecoverAfterRelaunchAndRestoreToMarkdownFile() throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }

        let store = MarkdownNoteStore(directory: directory)
        let note = try XCTUnwrap(store.create(contents: "# Plan\n\nFirst draft"))
        store.update(id: note.id, contents: "# Plan\n\nRecovered draft")

        let relaunched = MarkdownNoteStore(directory: directory)
        XCTAssertEqual(relaunched.recoveryCandidates, [note.id])
        XCTAssertEqual(relaunched.note(id: note.id)?.contents, "# Plan\n\nFirst draft")

        relaunched.restoreRecovery(for: note.id)
        XCTAssertEqual(relaunched.note(id: note.id)?.contents, "# Plan\n\nRecovered draft")
        XCTAssertFalse(relaunched.recoveryCandidates.contains(note.id))

        let reloaded = MarkdownNoteStore(directory: directory)
        XCTAssertEqual(reloaded.note(id: note.id)?.contents, "# Plan\n\nRecovered draft")

        let exportURL = directory.appendingPathComponent("exported.md")
        try reloaded.export(id: note.id, to: exportURL)
        XCTAssertEqual(try String(contentsOf: exportURL, encoding: .utf8), "# Plan\n\nRecovered draft")
    }

    func testDiscardedRecoveryDraftDoesNotBlockEditing() throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let recoveryDirectory = directory.appendingPathComponent("Recovery", isDirectory: true)
        try FileManager.default.createDirectory(at: recoveryDirectory, withIntermediateDirectories: true)
        let id = "safe-note"
        let draft = RecoveryDraftFixture(id: id, contents: "draft", savedAt: Date())
        let data = try JSONEncoder().encode(draft)
        try data.write(to: recoveryDirectory.appendingPathComponent(id).appendingPathExtension("json"))

        let store = MarkdownNoteStore(directory: directory)
        XCTAssertTrue(store.recoveryCandidates.contains(id))
        store.discardRecovery(for: id)
        XCTAssertFalse(store.recoveryCandidates.contains(id))
        store.update(id: id, contents: "accepted after discard")
        XCTAssertEqual(store.note(id: id)?.contents, "accepted after discard")
    }

    func testRejectsOversizedEditsAndUnsafeRecoveryIdentifiers() throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = MarkdownNoteStore(directory: directory)
        let note = try XCTUnwrap(store.create(contents: "small"))
        store.update(id: note.id, contents: String(repeating: "x", count: 2 * 1024 * 1024 + 1))
        XCTAssertEqual(store.note(id: note.id)?.contents, "small")
        XCTAssertNotNil(store.error)

        let recoveryDirectory = directory.appendingPathComponent("Recovery", isDirectory: true)
        let unsafe = RecoveryDraftFixture(id: "../../outside", contents: "unsafe", savedAt: Date())
        try JSONEncoder().encode(unsafe).write(to: recoveryDirectory.appendingPathComponent("unsafe.json"))
        let reloaded = MarkdownNoteStore(directory: directory)
        XCTAssertFalse(reloaded.recoveryCandidates.contains("../../outside"))
        XCTAssertFalse(FileManager.default.fileExists(atPath: directory.deletingLastPathComponent()
            .appendingPathComponent("outside.md").path))
    }

    private func makeTemporaryDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("KebloopNotesTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    private struct RecoveryDraftFixture: Codable {
        let id: String
        let contents: String
        let savedAt: Date
    }
}
