import Foundation
import XCTest
@testable import Kebloop

@MainActor
final class LocalAssistantServiceTests: XCTestCase {
    func testSendStreamsResponsesAndKeepsConversationHistory() async {
        let backend = FakeLocalAssistantBackend { prompt, update in
            update("Reply to \(prompt)")
            return "Reply to \(prompt)"
        }
        let service = LocalAssistantService(backend: backend)

        service.send("  First question  ")
        await waitUntil { !service.isResponding }

        XCTAssertEqual(service.messages.map(\.text), ["First question", "Reply to First question"])
        XCTAssertEqual(service.messages.map(\.role), [.user, .assistant])
        XCTAssertNil(service.status)

        service.send("Second question")
        await waitUntil { !service.isResponding }

        XCTAssertEqual(service.messages.map(\.text), [
            "First question", "Reply to First question", "Second question", "Reply to Second question"
        ])
    }

    func testUnavailableModelSurfacesAvailabilityMessageWithoutStartingGeneration() {
        let backend = FakeLocalAssistantBackend(availabilityMessage: "Enable Apple Intelligence to continue.")
        let service = LocalAssistantService(backend: backend)

        service.send("Hello")

        XCTAssertTrue(service.messages.isEmpty)
        XCTAssertFalse(service.isResponding)
        XCTAssertEqual(service.availabilityMessage, "Enable Apple Intelligence to continue.")
        XCTAssertEqual(service.status, "Enable Apple Intelligence to continue.")
    }

    func testCancelAndResetIgnoreLateStreamUpdates() async {
        let backend = FakeLocalAssistantBackend()
        let service = LocalAssistantService(backend: backend)

        service.send("Cancel me")
        await waitUntil { backend.hasPendingResponse }
        service.cancel()
        backend.emit("late update")
        backend.complete("late response")
        await Task.yield()

        XCTAssertEqual(service.messages.map(\.text), ["Cancel me"])
        XCTAssertFalse(service.isResponding)
        XCTAssertEqual(service.status, "Response cancelled.")
        XCTAssertEqual(backend.cancelCount, 1)

        service.send("Reset me")
        await waitUntil { backend.hasPendingResponse }
        service.reset()
        backend.emit("late update after reset")
        backend.complete("late response after reset")
        await Task.yield()

        XCTAssertTrue(service.messages.isEmpty)
        XCTAssertFalse(service.isResponding)
        XCTAssertNil(service.status)
        XCTAssertEqual(backend.resetCount, 1)
    }

    func testGenerationFailureIsShownAsStatus() async {
        let backend = FakeLocalAssistantBackend { _, _ in
            throw FakeError.failed
        }
        let service = LocalAssistantService(backend: backend)

        service.send("Hello")
        await waitUntil { !service.isResponding }

        XCTAssertEqual(service.status, "Local Assistant couldn't respond: The fixture failed.")
        XCTAssertEqual(service.messages.last?.text, "")
    }

    func testResetArchivesNonemptyConversationAndStartsANewID() async {
        let backend = FakeLocalAssistantBackend { prompt, update in
            update("Reply to \(prompt)")
            return "Reply to \(prompt)"
        }
        let history = LocalAssistantHistoryStore(storageURL: nil)
        let service = LocalAssistantService(tools: LocalAssistantTools(), backend: backend, history: history)
        let firstID = service.currentConversationID

        service.send("First conversation")
        await waitUntil { !service.isResponding }
        service.reset()

        XCTAssertNotEqual(service.currentConversationID, firstID)
        XCTAssertTrue(service.messages.isEmpty)
        XCTAssertEqual(history.conversations.map(\.id), [firstID])
        XCTAssertEqual(history.conversations.first?.title, "First conversation")
        XCTAssertEqual(history.conversations.first?.messages.map(\.text), ["First conversation", "Reply to First conversation"])
    }

    func testResumeRestoresMessagesAndBackendContext() async {
        let backend = FakeLocalAssistantBackend { prompt, update in
            update("Reply to \(prompt)")
            return "Reply to \(prompt)"
        }
        let history = LocalAssistantHistoryStore(storageURL: nil)
        let service = LocalAssistantService(tools: LocalAssistantTools(), backend: backend, history: history)
        service.send("Saved question")
        await waitUntil { !service.isResponding }
        let savedID = service.currentConversationID
        let savedMessages = service.messages

        service.reset()
        service.send("Temporary question")
        await waitUntil { !service.isResponding }
        XCTAssertNotEqual(service.currentConversationID, savedID)

        XCTAssertTrue(service.resumeConversation(id: savedID))
        XCTAssertEqual(service.currentConversationID, savedID)
        XCTAssertEqual(service.messages, savedMessages)
        XCTAssertEqual(backend.restoredMessages, savedMessages)
    }

    func testActiveConversationDeletionCancelsAndCannotResurrectHistory() async {
        let backend = FakeLocalAssistantBackend()
        let history = LocalAssistantHistoryStore(storageURL: nil)
        let service = LocalAssistantService(tools: LocalAssistantTools(), backend: backend, history: history)
        let deletedID = service.currentConversationID
        service.send("Delete while pending")
        await waitUntil { backend.hasPendingResponse }
        XCTAssertEqual(history.conversation(id: deletedID)?.messages.first?.text, "Delete while pending")

        service.removeConversation(id: deletedID)
        let newID = service.currentConversationID
        backend.emit("late update")
        backend.complete("late result")
        await Task.yield()

        XCTAssertNotEqual(newID, deletedID)
        XCTAssertNil(history.conversation(id: deletedID))
        XCTAssertFalse(history.conversations.contains(where: { $0.id == deletedID }))
        XCTAssertTrue(service.messages.isEmpty)
    }

    func testCancelPersistsPartialConversation() async {
        let backend = FakeLocalAssistantBackend()
        let history = LocalAssistantHistoryStore(storageURL: nil)
        let service = LocalAssistantService(tools: LocalAssistantTools(), backend: backend, history: history)
        let conversationID = service.currentConversationID
        service.send("Keep this question")
        await waitUntil { backend.hasPendingResponse }
        backend.emit("partial answer")

        service.cancel()
        backend.complete("late answer")
        await Task.yield()

        XCTAssertEqual(history.conversation(id: conversationID)?.messages.map(\.text), ["Keep this question", "partial answer"])
    }

    private func waitUntil(
        file: StaticString = #filePath,
        line: UInt = #line,
        _ condition: @MainActor () -> Bool
    ) async {
        for _ in 0..<1_000 {
            if condition() { return }
            await Task.yield()
        }
        XCTFail("Condition did not become true", file: file, line: line)
    }
}

@MainActor
private final class FakeLocalAssistantBackend: LocalAssistantBackend {
    typealias Responder = (String, @escaping @MainActor (String) -> Void) async throws -> String

    private(set) var availabilityMessage: String?
    private(set) var cancelCount = 0
    private(set) var resetCount = 0
    private(set) var restoredMessages: [LocalAssistantMessage] = []
    private var responder: Responder?
    private var responseContinuation: CheckedContinuation<String, Error>?
    private var update: (@MainActor (String) -> Void)?

    var hasPendingResponse: Bool { responseContinuation != nil }

    init(availabilityMessage: String? = nil, responder: Responder? = nil) {
        self.availabilityMessage = availabilityMessage
        self.responder = responder
    }

    func refreshAvailability() {}

    func streamResponse(
        to prompt: String,
        onUpdate: @escaping @MainActor (String) -> Void
    ) async throws -> String {
        if let responder {
            return try await responder(prompt, onUpdate)
        }
        return try await withCheckedThrowingContinuation { continuation in
            responseContinuation = continuation
            update = onUpdate
        }
    }

    func cancel() {
        cancelCount += 1
    }

    func reset() {
        resetCount += 1
        availabilityMessage = nil
    }

    func restore(messages: [LocalAssistantMessage]) {
        restoredMessages = messages
    }

    func emit(_ text: String) {
        update?(text)
    }

    func complete(_ text: String) {
        responseContinuation?.resume(returning: text)
        responseContinuation = nil
        update = nil
    }
}

private enum FakeError: LocalizedError {
    case failed

    var errorDescription: String? { "The fixture failed." }
}
