import Foundation

/// A deterministic, local description of an installed app for assistant tool use.
/// It only reads the metadata captured by `ApplicationIndex`; matching never scans
/// the filesystem or consults the workspace at request time.
struct AssistantApplicationIdentity: Sendable {
    let app: IndexedApplication
    let customAlias: String?
    let aliases: [String]

    init(app: IndexedApplication, customAlias: String? = nil) {
        self.app = app
        self.customAlias = Self.cleaned(customAlias)

        var values = [
            app.name,
            app.displayName,
            app.bundleName,
            app.executableName,
            Self.applicationBundleName(at: app.path),
            customAlias
        ].compactMap(Self.cleaned)

        if Self.isVisualStudioCode(app) {
            values.append(contentsOf: ["Visual Studio Code", "VS Code", "VSCode", "Code"])
        }

        var seen = Set<String>()
        aliases = values.filter { seen.insert(Self.normalized($0)).inserted }
    }

    /// Returns whether this app can match a name or a safe partial identity.
    /// Use `resolve` when selecting an app, since it also checks ambiguity.
    func matches(query: String) -> Bool {
        let queryKeys = Self.exactKeys(for: query)
        guard !queryKeys.isEmpty else { return false }
        if !queryKeys.isDisjoint(with: exactKeys) { return true }
        return Self.hasPartialMatch(query: query, in: searchableIdentities)
    }

    /// Returns exact matches first. If there are no exact matches, only safe
    /// partial or acronym matches are returned; callers should act only when
    /// exactly one app remains.
    static func resolve(
        query: String,
        applications: [IndexedApplication],
        customAliases: [String: String]
    ) -> [IndexedApplication] {
        let ordered = applications.sorted { lhs, rhs in
            if lhs.path != rhs.path { return lhs.path < rhs.path }
            return lhs.name < rhs.name
        }
        let identities = ordered.map { app in
            AssistantApplicationIdentity(app: app, customAlias: customAliases[app.path])
        }
        let exactMatches = identities.filter { identity in
            !Self.exactKeys(for: query).isDisjoint(with: identity.exactKeys)
        }
        if !exactMatches.isEmpty {
            return Self.uniqueApplications(exactMatches.map(\.app))
        }

        let partialMatches = identities.filter {
            Self.hasPartialMatch(query: query, in: $0.searchableIdentities)
        }
        return Self.uniqueApplications(partialMatches.map(\.app))
    }

    private var exactKeys: Set<String> {
        var values = aliases
        if let bundleIdentifier = Self.cleaned(app.bundleIdentifier) {
            // Bundle IDs are useful as an exact identifier, but their suffixes
            // are not names: `com.example.Code` must not match every query for
            // “code”.
            values.append(bundleIdentifier)
        }
        return Set(values.flatMap(Self.exactKeys(for:)))
    }

    private var searchableIdentities: [String] {
        var values = aliases
        if Self.isVisualStudioCode(app) {
            values.append(contentsOf: ["Visual Studio Code", "VS Code", "VSCode", "Code"])
        }
        return values
    }

    private static func hasPartialMatch(query: String, in identities: [String]) -> Bool {
        let normalizedQuery = normalized(query)
        let compactQuery = compact(normalizedQuery)
        guard compactQuery.count >= 2 else { return false }

        return identities.contains { identity in
            let normalizedIdentity = normalized(identity)
            let compactIdentity = compact(normalizedIdentity)
            if normalizedIdentity.contains(normalizedQuery) || compactIdentity.contains(compactQuery) {
                return true
            }
            return acronym(of: normalizedIdentity) == compactQuery
        }
    }

    private static func exactKeys(for value: String) -> Set<String> {
        let normalizedValue = normalized(value)
        guard !normalizedValue.isEmpty else { return [] }
        return Set([normalizedValue, compact(normalizedValue)])
    }

    private static func normalized(_ value: String) -> String {
        value
            .folding(options: [.diacriticInsensitive, .caseInsensitive], locale: Locale(identifier: "en_US_POSIX"))
            .split(whereSeparator: { !$0.isLetter && !$0.isNumber })
            .joined(separator: " ")
    }

    private static func compact(_ value: String) -> String {
        value.filter { $0.isLetter || $0.isNumber }
    }

    private static func acronym(of normalizedValue: String) -> String {
        normalizedValue.split(separator: " ").compactMap(\.first).map(String.init).joined()
    }

    private static func applicationBundleName(at path: String) -> String? {
        let name = URL(fileURLWithPath: path).deletingPathExtension().lastPathComponent
        return cleaned(name)
    }

    private static func cleaned(_ value: String?) -> String? {
        guard let value = value?.trimmingCharacters(in: .whitespacesAndNewlines), !value.isEmpty else {
            return nil
        }
        return value
    }

    private static func isVisualStudioCode(_ app: IndexedApplication) -> Bool {
        let bundleName = applicationBundleName(at: app.path)
        let names = [app.name, app.displayName, app.bundleName, bundleName].compactMap(cleaned)
        let normalizedNames = names.map(normalized)
        let normalizedBundleID = app.bundleIdentifier.map(normalized)

        guard !normalizedNames.contains(where: { $0.contains("insider") }) else { return false }
        guard !normalizedNames.contains("visual studio") else { return false }

        return normalizedBundleID == normalized("com.microsoft.VSCode")
            || normalizedNames.contains(normalized("Visual Studio Code"))
    }

    private static func uniqueApplications(_ applications: [IndexedApplication]) -> [IndexedApplication] {
        var seen = Set<String>()
        return applications.filter { seen.insert($0.path).inserted }
    }
}
