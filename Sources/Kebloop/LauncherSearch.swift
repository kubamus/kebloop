import Foundation
import KebloopCore

enum LauncherSearch {
    static let settingsAliases = ["settings", "preferences", "system settings", "system preferences", "ustawienia", "preferencje", "ustawienia systemowe"]

    static func ranked(query: String, entries: [AppSearchEntry], usage: [String: Int],
                       systemSettingsIDs: Set<String>) -> [AppSearchEntry] {
        var results = AppSearch().ranked(query: query, entries: entries, usage: usage)
        let normalized = query.trimmingCharacters(in: .whitespacesAndNewlines)
            .folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "en_US_POSIX"))
        guard normalized.count >= 2,
              settingsAliases.contains(where: { $0.hasPrefix(normalized) }),
              let kebloopIndex = results.firstIndex(where: { $0.id == "command:settings" }),
              let systemIndex = results.firstIndex(where: { systemSettingsIDs.contains($0.id) }),
              systemIndex > kebloopIndex else { return results }
        let systemSettings = results.remove(at: systemIndex)
        results.insert(systemSettings, at: kebloopIndex)
        return results
    }
}
