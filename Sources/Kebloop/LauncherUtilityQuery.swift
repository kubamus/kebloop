import Foundation

enum LauncherUtilityQuery {
    static func dictionaryWord(in query: String) -> String? {
        argument(in: query, keywords: ["define", "dict", "dictionary", "lookup", "słownik"])
    }

    static func noteSearch(in query: String) -> String? {
        argument(in: query, keywords: ["notes", "note", "notatki"])
    }

    private static func argument(in query: String, keywords: Set<String>) -> String? {
        let parts = query.trimmingCharacters(in: .whitespacesAndNewlines)
            .split(maxSplits: 1, whereSeparator: { $0.isWhitespace })
        guard parts.count == 2, keywords.contains(parts[0].lowercased()) else { return nil }
        let text = String(parts[1]).trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }
}
