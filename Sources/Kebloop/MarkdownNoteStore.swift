import Combine
import Foundation

struct MarkdownNote: Identifiable, Hashable {
    let id: String
    var contents: String
    var modifiedAt: Date

    var title: String {
        for line in contents.split(whereSeparator: \.isNewline) {
            let value = line.trimmingCharacters(in: .whitespacesAndNewlines)
            if value.isEmpty { continue }
            let heading = value.drop(while: { $0 == "#" }).trimmingCharacters(in: .whitespaces)
            let title = heading.isEmpty ? value : heading
            return String(title.prefix(80))
        }
        return "Untitled Note"
    }
}

@MainActor
final class MarkdownNoteStore: ObservableObject {
    @Published private(set) var notes: [MarkdownNote] = []
    @Published private(set) var recoveryCandidates: Set<String> = []
    @Published private(set) var error: String?

    private struct RecoveryDraft: Codable {
        let id: String
        let contents: String
        let savedAt: Date
    }

    private let directory: URL
    private let recoveryDirectory: URL
    private let fileManager: FileManager
    private var saveTasks: [String: Task<Void, Never>] = [:]
    private var revisions: [String: UUID] = [:]

    private nonisolated static let maximumNoteBytes = 2 * 1024 * 1024
    private nonisolated static let maximumRecoveryBytes = maximumNoteBytes * 6 + 4_096
    private nonisolated static let maximumLoadedNotes = 500
    private nonisolated static let maximumLoadedBytes = 25 * 1024 * 1024

    init(directory: URL = MarkdownNoteStore.defaultDirectory, fileManager: FileManager = .default) {
        self.directory = directory
        recoveryDirectory = directory.appendingPathComponent("Recovery", isDirectory: true)
        self.fileManager = fileManager
        load()
    }

    nonisolated static var defaultDirectory: URL {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support", isDirectory: true)
        return support.appendingPathComponent("Kebloop/Notes", isDirectory: true)
    }

    var selectedNotesDirectory: URL { directory }

    @discardableResult
    func create(contents: String = "# New Note\n\n") -> MarkdownNote? {
        guard notes.count < Self.maximumLoadedNotes,
              canStore(contents.utf8.count, excluding: nil) else {
            error = L("Kebloop's notes folder has reached its local storage limit.")
            return nil
        }
        let note = MarkdownNote(id: UUID().uuidString.lowercased(), contents: contents, modifiedAt: Date())
        notes.append(note)
        notes.sort { $0.modifiedAt > $1.modifiedAt }
        do {
            try ensureDirectories()
            try Data(contents.utf8).write(to: noteURL(note.id), options: .atomic)
            error = nil
        } catch {
            notes.removeAll { $0.id == note.id }
            self.error = error.localizedDescription
        }
        return note
    }

    func update(id: String, contents: String) {
        guard let index = notes.firstIndex(where: { $0.id == id }) else { return }
        guard !recoveryCandidates.contains(id) else {
            error = L("Restore or discard the recovered draft before editing this note.")
            return
        }
        guard contents.utf8.count <= Self.maximumNoteBytes, canStore(contents.utf8.count, excluding: id) else {
            error = L("This note would exceed Kebloop's local note storage limit.")
            return
        }
        notes[index].contents = contents
        notes[index].modifiedAt = Date()
        notes.sort { $0.modifiedAt > $1.modifiedAt }
        do {
            guard let updated = notes.first(where: { $0.id == id }) else { return }
            try writeRecoveryDraft(for: updated)
            error = nil
        } catch { self.error = error.localizedDescription }

        saveTasks[id]?.cancel()
        let revision = UUID()
        revisions[id] = revision
        saveTasks[id] = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 350_000_000)
            guard !Task.isCancelled, let self, self.revisions[id] == revision else { return }
            _ = self.persist(id: id)
        }
    }

    @discardableResult
    func flush(id: String) -> Bool {
        saveTasks[id]?.cancel()
        saveTasks[id] = nil
        revisions[id] = UUID()
        return persist(id: id)
    }

    func flushAll() {
        for task in saveTasks.values { task.cancel() }
        let dirtyIDs = Array(saveTasks.keys)
        saveTasks.removeAll()
        for id in dirtyIDs { _ = persist(id: id) }
    }

    func restoreRecovery(for id: String) {
        guard Self.isSafeID(id), let url = recoveryURL(id),
              let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey]),
              values.isRegularFile == true,
              (values.fileSize ?? .max) <= Self.maximumRecoveryBytes,
              let data = try? Data(contentsOf: url),
              let draft = try? JSONDecoder().decode(RecoveryDraft.self, from: data),
              draft.id == id,
              draft.contents.utf8.count <= Self.maximumNoteBytes,
              let index = notes.firstIndex(where: { $0.id == id }),
              canStore(draft.contents.utf8.count, excluding: id) else {
            error = L("The recovery draft is invalid or exceeds the local storage limit.")
            return
        }
        notes[index].contents = draft.contents
        notes[index].modifiedAt = draft.savedAt
        _ = flush(id: id)
    }

    func discardRecovery(for id: String) {
        guard Self.isSafeID(id) else { return }
        guard let url = recoveryURL(id) else { return }
        do {
            if fileManager.fileExists(atPath: url.path) { try fileManager.removeItem(at: url) }
            recoveryCandidates.remove(id)
            error = nil
        } catch { self.error = error.localizedDescription }
    }

    func remove(id: String) {
        saveTasks[id]?.cancel()
        saveTasks[id] = nil
        revisions[id] = nil
        if let index = notes.firstIndex(where: { $0.id == id }) {
            do {
                guard persist(id: id) else { return }
                let noteFile = noteURL(id)
                if fileManager.fileExists(atPath: noteFile.path) { try fileManager.removeItem(at: noteFile) }
                if let draft = recoveryURL(id), fileManager.fileExists(atPath: draft.path) { try fileManager.removeItem(at: draft) }
            } catch {
                self.error = error.localizedDescription
                return
            }
            notes.remove(at: index)
        }
        recoveryCandidates.remove(id)
    }

    func export(id: String, to destination: URL) throws {
        guard let note = notes.first(where: { $0.id == id }) else { throw MarkdownNoteError.noteMissing }
        try note.contents.write(to: destination, atomically: true, encoding: .utf8)
    }

    func note(id: String) -> MarkdownNote? { notes.first { $0.id == id } }

    private func load() {
        do {
            try ensureDirectories()
            let urls = try fileManager.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.contentModificationDateKey], options: [.skipsHiddenFiles])
            var loaded: [String: MarkdownNote] = [:]
            var loadedBytes = 0
            var loadWarning: String?
            for url in urls.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) where url.pathExtension.lowercased() == "md" {
                guard Self.isSafeID(url.deletingPathExtension().lastPathComponent),
                      let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey, .contentModificationDateKey]),
                      values.isRegularFile == true else { continue }
                let byteCount = values.fileSize ?? 0
                guard byteCount <= Self.maximumNoteBytes,
                      loaded.count < Self.maximumLoadedNotes,
                      loadedBytes + byteCount <= Self.maximumLoadedBytes else {
                    loadWarning = L("Some notes exceeded the local storage limits and were not opened.")
                    continue
                }
                guard let contents = try? String(contentsOf: url, encoding: .utf8) else { continue }
                let id = url.deletingPathExtension().lastPathComponent
                let modified = values.contentModificationDate ?? Date.distantPast
                loaded[id] = MarkdownNote(id: id, contents: contents, modifiedAt: modified)
                loadedBytes += byteCount
            }

            let drafts = try fileManager.contentsOfDirectory(at: recoveryDirectory, includingPropertiesForKeys: nil, options: [.skipsHiddenFiles])
            for url in drafts where url.pathExtension == "json" {
                guard let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey]),
                      values.isRegularFile == true,
                      (values.fileSize ?? .max) <= Self.maximumRecoveryBytes else { continue }
                guard let data = try? Data(contentsOf: url),
                      let draft = try? JSONDecoder().decode(RecoveryDraft.self, from: data),
                      Self.isSafeID(draft.id),
                      url.lastPathComponent == draft.id + ".json" else { continue }
                let persistedNote = loaded[draft.id]
                let hasUnsavedChanges: Bool
                if let persistedNote {
                    // Any content difference may represent an edit that had not reached disk before a crash.
                    // Timestamps are too coarse to safely discard a draft written just after a save.
                    hasUnsavedChanges = persistedNote.contents != draft.contents
                } else {
                    hasUnsavedChanges = true
                }
                if hasUnsavedChanges {
                    let draftBytes = draft.contents.utf8.count
                    let oldBytes = loaded[draft.id]?.contents.utf8.count ?? 0
                    let updatedBytes = loadedBytes - oldBytes + draftBytes
                    guard draftBytes <= Self.maximumNoteBytes,
                          loaded[draft.id] != nil || loaded.count < Self.maximumLoadedNotes,
                          updatedBytes <= Self.maximumLoadedBytes else {
                        loadWarning = L("Some recovery drafts exceeded the local storage limits and were not opened.")
                        continue
                    }
                    recoveryCandidates.insert(draft.id)
                    if loaded[draft.id] == nil {
                        loaded[draft.id] = MarkdownNote(id: draft.id, contents: "", modifiedAt: draft.savedAt)
                    }
                    loadedBytes = updatedBytes
                } else {
                    do { try fileManager.removeItem(at: url) }
                    catch { loadWarning = error.localizedDescription }
                }
            }
            notes = loaded.values.sorted { $0.modifiedAt > $1.modifiedAt }
            error = loadWarning
        } catch { self.error = error.localizedDescription }
    }

    private func persist(id: String) -> Bool {
        guard let note = notes.first(where: { $0.id == id }) else { return false }
        do {
            guard note.contents.utf8.count <= Self.maximumNoteBytes else { throw MarkdownNoteError.noteTooLarge }
            try ensureDirectories()
            try note.contents.write(to: noteURL(id), atomically: true, encoding: .utf8)
            if let draft = recoveryURL(id), fileManager.fileExists(atPath: draft.path) { try fileManager.removeItem(at: draft) }
            recoveryCandidates.remove(id)
            error = nil
            return true
        } catch {
            self.error = error.localizedDescription
            return false
        }
    }

    private func writeRecoveryDraft(for note: MarkdownNote) throws {
        try ensureDirectories()
        let draft = RecoveryDraft(id: note.id, contents: note.contents, savedAt: note.modifiedAt)
        let data = try JSONEncoder().encode(draft)
        guard data.count <= Self.maximumRecoveryBytes else { throw MarkdownNoteError.noteTooLarge }
        try data.write(to: recoveryDirectory.appendingPathComponent(note.id + ".json"), options: .atomic)
    }

    private func ensureDirectories() throws {
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        try fileManager.createDirectory(at: recoveryDirectory, withIntermediateDirectories: true)
    }

    private func noteURL(_ id: String) -> URL { directory.appendingPathComponent(id).appendingPathExtension("md") }
    private func recoveryURL(_ id: String) -> URL? { recoveryDirectory.appendingPathComponent(id).appendingPathExtension("json") }

    private func canStore(_ newBytes: Int, excluding id: String?) -> Bool {
        let currentBytes = notes.lazy.filter { $0.id != id }.reduce(0) { $0 + $1.contents.utf8.count }
        return newBytes <= Self.maximumNoteBytes && currentBytes + newBytes <= Self.maximumLoadedBytes
    }

    private nonisolated static func isSafeID(_ id: String) -> Bool {
        !id.isEmpty && id.count <= 80 && id != "." && id != ".."
            && id.unicodeScalars.allSatisfy { CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-_" )).contains($0) }
    }
}

enum MarkdownNoteError: LocalizedError {
    case noteMissing
    case noteTooLarge
    var errorDescription: String? {
        switch self {
        case .noteMissing: "The note is no longer available."
        case .noteTooLarge: "The note is larger than Kebloop's local storage limit."
        }
    }
}
