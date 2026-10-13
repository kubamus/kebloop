import AppKit
import Combine
import Foundation

enum MarkdownNoteFindAction: Equatable {
    case show
    case next
    case previous

    var textFinderAction: NSTextFinder.Action {
        switch self {
        case .show: .showFindInterface
        case .next: .nextMatch
        case .previous: .previousMatch
        }
    }
}

struct MarkdownNoteFindRequest: Equatable {
    let revision: Int
    let action: MarkdownNoteFindAction
}

/// Owns the current note and the rules for moving between editing, preview, and launcher actions.
@MainActor
final class MarkdownNoteSession: ObservableObject {
    @Published private(set) var selectedNoteID: String?
    @Published private(set) var editingNoteID: String?
    @Published private(set) var draft = ""
    @Published private(set) var editorFocusRevision = 0
    @Published private(set) var findRequest: MarkdownNoteFindRequest?

    private var findRevision = 0

    let store: MarkdownNoteStore

    private let onClose: () -> Void
    private let onUnsavedDraftChanged: (Bool) -> Void

    init(
        store: MarkdownNoteStore,
        onClose: @escaping () -> Void,
        onUnsavedDraftChanged: @escaping (Bool) -> Void
    ) {
        self.store = store
        self.onClose = onClose
        self.onUnsavedDraftChanged = onUnsavedDraftChanged
    }

    var isEditing: Bool { editingNoteID != nil }
    var selectedNote: MarkdownNote? {
        guard let selectedNoteID else { return nil }
        return store.note(id: selectedNoteID)
    }
    var hasRecoveryDraft: Bool {
        guard let selectedNoteID else { return false }
        return store.recoveryCandidates.contains(selectedNoteID)
    }

    func open(initialNoteID: String?, initiallyEditing: Bool) {
        let requestedNoteID: String?
        if let initialNoteID, store.note(id: initialNoteID) != nil {
            requestedNoteID = initialNoteID
        } else if selectedNoteID == nil || store.note(id: selectedNoteID ?? "") == nil {
            requestedNoteID = store.notes.first?.id
        } else {
            requestedNoteID = selectedNoteID
        }

        if requestedNoteID != selectedNoteID {
            guard saveAndPreview() else { return }
            selectedNoteID = requestedNoteID
        }

        if initiallyEditing {
            guard !isEditing else { return }
            if selectedNoteID == nil {
                _ = createBlankNote()
            } else {
                _ = beginEditing()
            }
        }
    }

    @discardableResult
    func toggleEditing() -> Bool {
        if isEditing { return saveAndPreview() }
        return beginEditing()
    }

    @discardableResult
    func saveAndPreview() -> Bool {
        guard let editingNoteID else { return true }
        store.update(id: editingNoteID, contents: draft)
        guard store.note(id: editingNoteID)?.contents == draft,
              store.flush(id: editingNoteID) else {
            onUnsavedDraftChanged(true)
            return false
        }

        self.editingNoteID = nil
        findRequest = nil
        onUnsavedDraftChanged(false)
        return true
    }

    @discardableResult
    func escape() -> Bool {
        if isEditing { return saveAndPreview() }
        return close()
    }

    @discardableResult
    func close() -> Bool {
        guard saveAndPreview() else { return false }
        onClose()
        return true
    }

    @discardableResult
    func saveAndBrowse(_ onBrowse: () -> Void) -> Bool {
        guard saveAndPreview() else { return false }
        onBrowse()
        return true
    }

    @discardableResult
    func requestFind(_ action: MarkdownNoteFindAction) -> Bool {
        if !isEditing {
            guard action == .show, beginEditing() else { return false }
        }
        findRevision += 1
        findRequest = MarkdownNoteFindRequest(revision: findRevision, action: action)
        return true
    }

    @discardableResult
    func createBlankNote() -> Bool {
        guard saveAndPreview(), let note = store.create(contents: "") else { return false }
        selectedNoteID = note.id
        return beginEditing()
    }

    func setDraft(_ value: String) {
        guard isEditing, value != draft, let editingNoteID else { return }
        draft = value
        store.update(id: editingNoteID, contents: value)
        let rejected = store.note(id: editingNoteID)?.contents != value || store.error != nil
        onUnsavedDraftChanged(rejected)
    }

    @discardableResult
    func prepareDelete() -> Bool {
        saveAndPreview() && selectedNote != nil
    }

    @discardableResult
    func deleteSelected() -> Bool {
        guard !isEditing, let noteID = selectedNoteID else { return false }
        store.remove(id: noteID)
        guard store.note(id: noteID) == nil else {
            return false
        }
        selectedNoteID = store.notes.first?.id
        onUnsavedDraftChanged(false)
        return true
    }

    @discardableResult
    func restoreRecovery() -> Bool {
        guard let selectedNoteID, store.recoveryCandidates.contains(selectedNoteID) else { return false }
        store.restoreRecovery(for: selectedNoteID)
        guard !store.recoveryCandidates.contains(selectedNoteID) else { return false }
        onUnsavedDraftChanged(false)
        return true
    }

    @discardableResult
    func discardRecovery() -> Bool {
        guard let selectedNoteID, store.recoveryCandidates.contains(selectedNoteID) else { return false }
        store.discardRecovery(for: selectedNoteID)
        let discarded = !store.recoveryCandidates.contains(selectedNoteID)
        if discarded { onUnsavedDraftChanged(false) }
        return discarded
    }

    func reconcileSelection() {
        if let selectedNoteID, store.note(id: selectedNoteID) != nil { return }
        guard saveAndPreview() else { return }
        editingNoteID = nil
        draft = ""
        selectedNoteID = store.notes.first?.id
        onUnsavedDraftChanged(false)
    }

    private func beginEditing() -> Bool {
        guard let note = selectedNote,
              !store.recoveryCandidates.contains(note.id) else { return false }
        editingNoteID = note.id
        draft = note.contents
        editorFocusRevision += 1
        return true
    }
}
