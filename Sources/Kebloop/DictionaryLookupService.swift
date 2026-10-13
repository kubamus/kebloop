import AppKit
import CoreServices
import Foundation

enum DictionaryLookupService {
    static func definition(for rawWord: String) -> String? {
        let word = rawWord.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !word.isEmpty, word.utf16.count <= 256 else { return nil }
        guard let definition = DCSCopyTextDefinition(nil, word as CFString, CFRange(location: 0, length: word.utf16.count))?.takeRetainedValue() as String? else { return nil }
        let trimmed = definition.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    @discardableResult
    static func openInDictionary(_ rawWord: String) -> Bool {
        let word = rawWord.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !word.isEmpty,
              let encoded = word.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed),
              let url = URL(string: "dict://\(encoded)") else { return false }
        return NSWorkspace.shared.open(url)
    }
}
