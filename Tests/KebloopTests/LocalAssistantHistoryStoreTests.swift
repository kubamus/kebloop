import Foundation
import XCTest
@testable import Kebloop

@MainActor
final class LocalAssistantHistoryStoreTests: XCTestCase {
    func testConversationPersistsAndCanBeLoadedByANewStore() throws {
        let storageURL = makeStorageURL()
        defer { try? FileManager.default.removeItem(at: storageURL.deletingLastPathComponent()) }
        let firstDate = Date(timeIntervalSince1970: 100)
        let secondDate = Date(timeIntervalSince1970: 200)
        let conversationID = UUID()
        let userMessage = LocalAssistantMessage(role: .user, text: "  Find a local bakery  ")
        let assistantMessage = LocalAssistantMessage(role: .assistant, text: "Here are three nearby bakeries.")

        let store = LocalAssistantHistoryStore(storageURL: storageURL)
        store.upsert(id: conversationID, messages: [userMessage], now: firstDate)
        store.upsert(id: conversationID, messages: [userMessage, assistantMessage], now: secondDate)

        let loaded = LocalAssistantHistoryStore(storageURL: storageURL)
        let conversation = try XCTUnwrap(loaded.conversation(id: conversationID))
        XCTAssertEqual(conversation.title, "Find a local bakery")
        XCTAssertEqual(conversation.createdAt, firstDate)
        XCTAssertEqual(conversation.updatedAt, secondDate)
        XCTAssertEqual(conversation.messages, [userMessage, assistantMessage])
    }

    func testEmptyConversationsAreNotSavedAndDeletionSurvivesReload() throws {
        let storageURL = makeStorageURL()
        defer { try? FileManager.default.removeItem(at: storageURL.deletingLastPathComponent()) }
        let store = LocalAssistantHistoryStore(storageURL: storageURL)
        let emptyID = UUID()
        let savedID = UUID()

        store.upsert(id: emptyID, messages: [])
        store.upsert(id: savedID, messages: [LocalAssistantMessage(role: .user, text: "Persist me")])
        XCTAssertNil(store.conversation(id: emptyID))

        store.removeConversation(id: savedID)
        let loaded = LocalAssistantHistoryStore(storageURL: storageURL)
        XCTAssertTrue(loaded.conversations.isEmpty)
    }

    private func makeStorageURL() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("KebloopHistoryTests-\(UUID().uuidString)", isDirectory: true)
            .appendingPathComponent("history.json")
    }
}
