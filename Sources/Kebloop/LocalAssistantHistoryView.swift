import SwiftUI

struct LocalAssistantHistoryView: View {
    @ObservedObject var history: LocalAssistantHistoryStore
    let query: String
    var selectedID: UUID?
    var onOpen: (UUID) -> Void
    var onDelete: (UUID) -> Void

    static func matches(_ conversations: [LocalAssistantConversation], query: String) -> [LocalAssistantConversation] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return conversations.filter { conversation in
            query.isEmpty || conversation.title.localizedCaseInsensitiveContains(query)
                || conversation.messages.contains { $0.text.localizedCaseInsensitiveContains(query) }
        }
    }

    var body: some View {
        let conversations = Self.matches(history.conversations, query: query)
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 8) {
                    if let error = history.error {
                        Text(error).font(.caption).foregroundStyle(.red)
                    }
                    if conversations.isEmpty {
                        Text(history.conversations.isEmpty ? L("No conversations yet.") : L("No matching conversations."))
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .center)
                            .padding(.vertical, 24)
                    }
                    ForEach(conversations) { conversation in
                        HStack(alignment: .top, spacing: 12) {
                            Button { onOpen(conversation.id) } label: {
                                VStack(alignment: .leading, spacing: 5) {
                                    Text(conversation.title).font(.headline).lineLimit(1)
                                    if let preview = conversation.messages.last(where: { !$0.text.isEmpty }) {
                                        Text(preview.text).font(.callout).foregroundStyle(.secondary).lineLimit(2)
                                    }
                                    Text(conversation.updatedAt.formatted(date: .abbreviated, time: .shortened))
                                        .font(.caption).foregroundStyle(.tertiary)
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(L("Open Conversation") + ": " + conversation.title)
                            Button(role: .destructive) { onDelete(conversation.id) } label: {
                                Image(systemName: "trash")
                            }
                            .buttonStyle(.borderless)
                            .accessibilityLabel(L("Delete Conversation") + ": " + conversation.title)
                        }
                        .padding(10)
                        .background(conversation.id == selectedID ? Color.accentColor.opacity(0.14) : Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 8))
                    }
                }
                .padding(12)
            }
            .onChange(of: selectedID) { _, id in
                if let id { proxy.scrollTo(id, anchor: .center) }
            }
        }
    }
}
