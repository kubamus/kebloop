import Foundation

struct RaycastImportItem: Identifiable {
    enum Kind: String { case snippet = "Snippet", quicklink = "Quicklink" }
    let id: String
    let kind: Kind
    let name: String
    let value: String
    let keyword: String
    let issue: String?
}

/// Reads Raycast's individual JSON exports, never executable configuration or encrypted backups.
enum RaycastMigration {
    private struct Record: Decodable {
        let name: String
        let text: String?
        let link: String?
        let url: String?
        let keyword: String?
    }
    static func preview(data: Data) throws -> [RaycastImportItem] {
        guard data.count <= 10_000_000 else { throw ConfigurationError.invalid("The export exceeds 10 MB.") }
        let records: [Record]
        do { records = try JSONDecoder().decode([Record].self, from: data) }
        catch { throw ConfigurationError.invalid("Choose the JSON from Raycast's Export Snippets or Export Quicklinks command. Encrypted .rayconfig backups are not supported.") }
        guard records.count <= 10_000 else { throw ConfigurationError.invalid("The export exceeds 10,000 items.") }
        return records.enumerated().map { offset, record in
            let kind: RaycastImportItem.Kind = record.text != nil ? .snippet : .quicklink
            let value = record.text ?? record.link ?? record.url ?? ""
            var issue: String?
            if record.text != nil && (record.link != nil || record.url != nil) { issue = "Ambiguous item with both text and a link." }
            else if kind == .snippet {
                // Raycast's dynamic grammar differs from Kebloop. Do not silently reinterpret tokens.
                if value.range(of: #"\{[^{}]+\}"#, options: .regularExpression) != nil { issue = "Dynamic placeholders require manual conversion before importing." }
                else {
                    do { try Snippet(name: record.name, template: value).validate() }
                    catch { issue = error.localizedDescription }
                }
            } else if record.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || Quicklink.destination(value) == nil {
                issue = "Unsupported link. Use http(s), an absolute file path, or a {query} URL template."
            }
            let keyword = kind == .quicklink ? (record.keyword ?? "").trimmingCharacters(in: .whitespacesAndNewlines).lowercased() : (record.keyword ?? "")
            if kind == .quicklink && keyword.contains(where: { $0.isWhitespace }) { issue = "The quicklink keyword must be a single word." }
            return RaycastImportItem(id: String(offset), kind: kind, name: record.name.trimmingCharacters(in: .whitespacesAndNewlines), value: value, keyword: keyword, issue: issue)
        }
    }

    @MainActor static func conflict(_ item: RaycastImportItem, snippets: SnippetStore, quicklinks: QuicklinkStore) -> String? {
        if let issue = item.issue { return issue }
        switch item.kind {
        case .snippet:
            if snippets.snippets.contains(where: { $0.name.caseInsensitiveCompare(item.name.trimmingCharacters(in: .whitespacesAndNewlines)) == .orderedSame || $0.template == item.value }) { return "An existing snippet has this name or text; it will be preserved." }
        case .quicklink:
            if quicklinks.links.contains(where: { $0.name.caseInsensitiveCompare(item.name.trimmingCharacters(in: .whitespacesAndNewlines)) == .orderedSame || $0.target == item.value }) { return "An existing quicklink has this name or target; it will be preserved." }
            if !item.keyword.isEmpty && (quicklinks.links.contains(where: { $0.keyword.caseInsensitiveCompare(item.keyword) == .orderedSame }) || quicklinks.webSearches.contains(where: { $0.keyword.caseInsensitiveCompare(item.keyword) == .orderedSame })) { return "This keyword is already in use; the existing item will be preserved." }
        }
        return nil
    }

    @MainActor static func importItems(_ items: [RaycastImportItem], selected: Set<String>, snippets: SnippetStore, quicklinks: QuicklinkStore) throws -> (imported: Int, skipped: Int) {
        guard Set(items.map(\.id)).count == items.count else { throw ConfigurationError.invalid("Duplicate import item identifiers.") }
        let candidates = items.filter { selected.contains($0.id) && $0.issue == nil }
        // Validate the entire chosen batch before the first persistent write.
        for item in candidates {
            switch item.kind {
            case .snippet:
                try Snippet(name: item.name.trimmingCharacters(in: .whitespacesAndNewlines), template: item.value).validate()
                guard item.value.range(of: #"\{[^{}]+\}"#, options: .regularExpression) == nil else { throw ConfigurationError.invalid("Dynamic snippet placeholders require conversion.") }
            case .quicklink:
                guard !item.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                      Quicklink.destination(item.value) != nil,
                      !item.keyword.contains(where: { $0.isWhitespace }) else { throw ConfigurationError.invalid("Invalid quicklink in the selected batch.") }
            }
        }
        var imported = 0, skipped = 0
        for item in items where selected.contains(item.id) {
            guard conflict(item, snippets: snippets, quicklinks: quicklinks) == nil else { skipped += 1; continue }
            switch item.kind {
            case .snippet:
                // Even if the user's typing monitor is already active, import does not activate new triggers.
                try snippets.save(name: item.name, template: item.value, keyword: nil)
            case .quicklink:
                if let error = quicklinks.save(name: item.name, target: item.value, keyword: item.keyword) { throw ConfigurationError.invalid(error) }
            }
            imported += 1
        }
        return (imported, skipped)
    }
}
