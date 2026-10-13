import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct MarkdownNotesView: View {
    @ObservedObject var store: MarkdownNoteStore
    let initialNoteID: String?
    let initiallyEditing: Bool
    let onBrowse: () -> Void

    @StateObject private var session: MarkdownNoteSession
    @State private var confirmsDelete = false

    init(
        store: MarkdownNoteStore,
        initialNoteID: String? = nil,
        initiallyEditing: Bool = false,
        onClose: @escaping () -> Void = {},
        onUnsavedDraftChanged: @escaping (Bool) -> Void = { _ in },
        onBrowse: @escaping () -> Void = {}
    ) {
        self.store = store
        self.initialNoteID = initialNoteID
        self.initiallyEditing = initiallyEditing
        self.onBrowse = onBrowse
        _session = StateObject(wrappedValue: MarkdownNoteSession(
            store: store,
            onClose: onClose,
            onUnsavedDraftChanged: onUnsavedDraftChanged
        ))
    }

    private var selectedNote: MarkdownNote? { session.selectedNote }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()

            if let note = selectedNote {
                if session.hasRecoveryDraft { recoveryActions }
                if session.isEditing {
                    MarkdownNoteEditor(
                        text: Binding(get: { session.draft }, set: session.setDraft),
                        focusRevision: session.editorFocusRevision,
                        findRequest: session.findRequest
                    )
                    .id(session.editingNoteID)
                    .accessibilityLabel(L("Markdown note editor"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    MarkdownPreview(source: note.contents)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                        .padding(16)
                }
            } else {
                ContentUnavailableView {
                    Label(L("No Notes"), systemImage: "note.text")
                } actions: {
                    Button(L("New Note"), action: createBlankNote)
                        .buttonStyle(.borderedProminent)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }

            Divider()
            footer
        }
        .frame(minWidth: 480, minHeight: 320)
        .confirmationDialog(
            L("Delete this note?"),
            isPresented: $confirmsDelete,
            titleVisibility: .visible
        ) {
            Button(L("Delete Note"), role: .destructive) { _ = session.deleteSelected() }
            Button(L("Cancel"), role: .cancel) {}
        } message: {
            Text(L("This permanently deletes the note and its recovery draft."))
        }
        .background {
            MarkdownNoteKeyMonitor(onKeyEvent: handleKeyEvent)
                .frame(width: 1, height: 1)
                .opacity(0)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
        .onAppear { session.open(initialNoteID: initialNoteID, initiallyEditing: initiallyEditing) }
        .onChange(of: initialNoteID) { _, noteID in
            session.open(initialNoteID: noteID, initiallyEditing: initiallyEditing)
        }
        .onChange(of: initiallyEditing) { _, editing in
            session.open(initialNoteID: initialNoteID, initiallyEditing: editing)
        }
        .onChange(of: store.notes.map(\.id)) { _, _ in session.reconcileSelection() }
        .onDisappear {
            if session.isEditing { _ = session.saveAndPreview() }
            store.flushAll()
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Text(selectedNote?.title ?? L("Notes"))
                .font(.headline)
                .lineLimit(1)
                .accessibilityAddTraits(.isHeader)

            Spacer(minLength: 8)

            Button(action: browseNotes) {
                Label(L("Notes"), systemImage: "list.bullet")
            }
            .help(L("Browse notes (⌘P)"))

            if selectedNote != nil {
                Button(session.isEditing ? L("Preview") : L("Edit")) {
                    _ = session.toggleEditing()
                }
                .disabled(!session.isEditing && session.hasRecoveryDraft)
                .help(session.isEditing ? L("Save and preview (⌘E)") : L("Edit Markdown (⌘E)"))

                Menu {
                    Button(L("Export Markdown…   ⇧⌘E"), action: exportSelected)
                    Button(L("Delete Note…   ⌘⌫"), role: .destructive, action: requestDelete)
                } label: {
                    Label(L("More"), systemImage: "ellipsis")
                }
                .menuStyle(.borderlessButton)
                .accessibilityLabel(L("More note actions"))
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    private var recoveryActions: some View {
        HStack(spacing: 12) {
            Text(L("Recovered draft available"))
                .font(.callout)
            Spacer()
            Button(L("Restore")) { _ = session.restoreRecovery() }
                .help(L("Restore recovered draft (⇧⌘R)"))
            Button(L("Discard")) { _ = session.discardRecovery() }
                .help(L("Discard recovered draft (⇧⌘D)"))
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
    }

    private var footer: some View {
        HStack(spacing: 8) {
            if let error = store.error {
                Text(error).lineLimit(1)
            } else if session.hasRecoveryDraft {
                Text(L("⇧⌘R Restore · ⇧⌘D Discard"))
            } else if session.isEditing {
                Text(L("⌘↩ Save · ⌘F Find · Esc Preview"))
            } else {
                Text(L("Return Edit · ⌘F Find · ⌘N New · ⌘P Notes"))
            }
            Spacer(minLength: 0)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 16)
        .padding(.vertical, 7)
        .accessibilityElement(children: .combine)
    }

    private func handleKeyEvent(_ event: NSEvent) -> Bool {
        guard !confirmsDelete else { return false }
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        let hasCommand = flags.contains(.command)
        guard !flags.contains(.option), !flags.contains(.control) else { return false }

        if event.keyCode == 53, !hasCommand, !flags.contains(.shift) {
            return session.escape()
        }

        if !hasCommand,
           flags.isDisjoint(with: [.shift, .option, .control]),
           (event.keyCode == 36 || event.keyCode == 76),
           !(event.window?.firstResponder is NSButton) {
            return session.isEditing ? false : session.toggleEditing()
        }

        guard hasCommand else { return false }
        let key = event.charactersIgnoringModifiers?.lowercased() ?? ""

        if flags.contains(.shift), key == "e" {
            exportSelected()
            return true
        }
        if flags.contains(.shift), key == "r" {
            _ = session.restoreRecovery()
            return true
        }
        if flags.contains(.shift), key == "d" {
            _ = session.discardRecovery()
            return true
        }
        if flags.contains(.shift), key == "g" {
            guard session.isEditing else { return false }
            _ = session.requestFind(.previous)
            return true
        }
        if (event.keyCode == 51 || event.keyCode == 117), !flags.contains(.shift) {
            guard !session.isEditing else { return false }
            requestDelete()
            return true
        }
        if flags.contains(.shift) { return false }

        switch key {
        case "e": _ = session.toggleEditing()
        case "s": _ = session.saveAndPreview()
        case "n": createBlankNote()
        case "p": browseNotes()
        case "f": _ = session.requestFind(.show)
        case "g":
            guard session.isEditing else { return false }
            _ = session.requestFind(.next)
        case "w": _ = session.close()
        default:
            if event.keyCode == 36 || event.keyCode == 76 {
                _ = session.saveAndPreview()
            } else {
                return false
            }
        }
        return true
    }

    private func createBlankNote() {
        _ = session.createBlankNote()
    }

    private func browseNotes() {
        _ = session.saveAndBrowse(onBrowse)
    }

    private func requestDelete() {
        guard session.prepareDelete() else { return }
        confirmsDelete = true
    }

    private func exportSelected() {
        guard session.saveAndPreview(), let note = session.selectedNote else { return }
        let panel = NSSavePanel()
        panel.title = L("Export Markdown Note")
        panel.nameFieldStringValue = Self.filename(for: note.title)
        panel.allowedContentTypes = [.plainText]
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do { try store.export(id: note.id, to: url) }
        catch { NSAlert(error: error).runModal() }
    }

    private static func filename(for title: String) -> String {
        let safe = title
            .replacingOccurrences(of: "/", with: "-")
            .replacingOccurrences(of: ":", with: "-")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return (safe.isEmpty ? "Note" : safe) + ".md"
    }
}
