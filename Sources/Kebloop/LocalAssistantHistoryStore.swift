import Combine
import Foundation

struct LocalAssistantConversation: Codable, Identifiable, Equatable {
    let id: UUID
    let createdAt: Date
    var updatedAt: Date
    var messages: [LocalAssistantMessage]

    var title: String {
        let firstPrompt = messages.first(where: { $0.role == .user })?.text
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard let firstPrompt, !firstPrompt.isEmpty else { return "New Chat" }
        return String(firstPrompt.prefix(100))
    }

    init(
        id: UUID = UUID(),
        createdAt: Date = Date(),
        updatedAt: Date = Date(),
        messages: [LocalAssistantMessage]
    ) {
        self.id = id
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.messages = messages
    }
}

@MainActor
final class LocalAssistantHistoryStore: ObservableObject {
    @Published private(set) var conversations: [LocalAssistantConversation] = []
    @Published private(set) var error: String?

    var errorMessage: String? { error }

    private let storageURL: URL?
    private let fileManager: FileManager
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder

    /// Creates a persistent store in Kebloop's Application Support directory.
    init() {
        storageURL = Self.defaultStorageURL(fileManager: .default)
        fileManager = .default
        encoder = JSONEncoder()
        decoder = JSONDecoder()
        load()
    }

    /// Pass `nil` to keep history in memory, or a URL to persist to an isolated file.
    init(storageURL: URL?, fileManager: FileManager = .default) {
        self.storageURL = storageURL
        self.fileManager = fileManager
        encoder = JSONEncoder()
        decoder = JSONDecoder()
        load()
    }

    func conversation(id: UUID) -> LocalAssistantConversation? {
        conversations.first(where: { $0.id == id })
    }

    func upsert(id: UUID, messages: [LocalAssistantMessage], now: Date = Date()) {
        guard !messages.isEmpty else { return }
        let existing = conversation(id: id)
        let conversation = LocalAssistantConversation(
            id: id,
            createdAt: existing?.createdAt ?? now,
            updatedAt: now,
            messages: Self.boundedMessages(messages)
        )
        conversations.removeAll { $0.id == id }
        conversations.append(conversation)
        conversations.sort { $0.updatedAt > $1.updatedAt }
        conversations = Array(conversations.prefix(Self.maximumConversations))
        save()
    }

    func removeConversation(id: UUID) {
        guard conversations.contains(where: { $0.id == id }) else { return }
        conversations.removeAll { $0.id == id }
        save()
    }

    private func load() {
        guard let storageURL, fileManager.fileExists(atPath: storageURL.path) else { return }
        do {
            let attributes = try fileManager.attributesOfItem(atPath: storageURL.path)
            let fileSize = (attributes[.size] as? NSNumber)?.intValue ?? 0
            guard fileSize <= Self.maximumFileSize else {
                error = "Couldn't read Local Assistant history: the history file exceeds the size limit."
                return
            }
            let data = try Data(contentsOf: storageURL)
            let decoded = try decoder.decode([LocalAssistantConversation].self, from: data)
            conversations = decoded
                .filter { !$0.messages.isEmpty }
                .sorted { $0.updatedAt > $1.updatedAt }
                .prefix(Self.maximumConversations)
                .map { conversation in
                    LocalAssistantConversation(
                        id: conversation.id,
                        createdAt: conversation.createdAt,
                        updatedAt: conversation.updatedAt,
                        messages: Self.boundedMessages(conversation.messages)
                    )
                }
            if conversations != decoded {
                save()
            }
        } catch {
            self.error = "Couldn't read Local Assistant history: \(error.localizedDescription)"
        }
    }

    private func save() {
        conversations = Array(conversations.prefix(Self.maximumConversations))
        guard let storageURL else {
            error = nil
            return
        }

        do {
            let directoryURL = storageURL.deletingLastPathComponent()
            try fileManager.createDirectory(
                at: directoryURL,
                withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700]
            )
            try fileManager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directoryURL.path)

            var savedConversations = Array(conversations.prefix(Self.maximumConversations))
            var data = try encoder.encode(savedConversations)
            while data.count > Self.maximumFileSize, savedConversations.count > 1 {
                savedConversations.removeLast()
                data = try encoder.encode(savedConversations)
            }
            try data.write(to: storageURL, options: .atomic)
            try fileManager.setAttributes([.posixPermissions: 0o600], ofItemAtPath: storageURL.path)
            conversations = savedConversations
            error = nil
        } catch {
            self.error = "Couldn't save Local Assistant history: \(error.localizedDescription)"
        }
    }

    private static func boundedMessages(_ messages: [LocalAssistantMessage]) -> [LocalAssistantMessage] {
        messages.suffix(maximumMessages).map { message in
            LocalAssistantMessage(
                id: message.id,
                role: message.role,
                text: String(message.text.prefix(maximumMessageLength))
            )
        }
    }

    private static func defaultStorageURL(fileManager: FileManager) -> URL? {
        guard let supportURL = fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else {
            return nil
        }
        return supportURL
            .appendingPathComponent("Kebloop", isDirectory: true)
            .appendingPathComponent("local-assistant-history.json", isDirectory: false)
    }

    private static let maximumConversations = 100
    private static let maximumMessages = 100
    private static let maximumMessageLength = 8_000
    private static let maximumFileSize = 8 * 1_024 * 1_024
}
