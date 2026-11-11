import AppKit
import Combine
import FinderSync
import KebloopCore
import SwiftUI

@MainActor
final class NewFileStore: ObservableObject {
    @Published var configuration = NewFileConfiguration()
    @Published var error: String?

    init() {
        do { configuration = try NewFileConfiguration.load() }
        catch { self.error = error.localizedDescription }
    }

    @discardableResult
    func save() -> Bool {
        do {
            try configuration.save()
            error = nil
            return true
        } catch {
            self.error = error.localizedDescription
            return false
        }
    }

    @discardableResult
    func update(_ change: (inout NewFileConfiguration) -> Void) -> Bool {
        let previous = configuration
        change(&configuration)
        guard save() else { configuration = previous; return false }
        return true
    }

    func create(_ type: NewFileType) {
        let panel = NSOpenPanel()
        panel.title = L("Choose a folder for the new file")
        panel.prompt = L("Create File")
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let directory = panel.url else { return }
        do {
            let url = try NewFileCreator.create(type: type, in: directory)
            error = nil
            NSWorkspace.shared.activateFileViewerSelecting([url])
        } catch { self.error = error.localizedDescription }
    }
}

struct NewFileView: View {
    @ObservedObject var store: NewFileStore
    var allowsCreation = false
    @State private var editing: NewFileType?
    @State private var extensionEnabled = FIFinderSyncController.isExtensionEnabled
    @State private var showExtensionWarning = false

    private var extensionSetupInstructions: String {
        L("Enable Kebloop New File in System Settings → General → Login Items & Extensions → Finder extensions (File Providers on macOS 27).")
    }

    var body: some View {
        Form {
            Section(L("Finder Integration")) {
                Text(extensionEnabled ? L("Kebloop New File is enabled in Finder.") : extensionSetupInstructions)
                Button(L("Open Extension Settings")) { FIFinderSyncController.showExtensionManagementInterface() }
                Text(L("In Finder, choose View → Customize Toolbar to add the New File button. The toolbar also works in iCloud Drive, where macOS may hide extension context menus."))
                    .font(.caption).foregroundStyle(.secondary)
                Toggle(L("Group right-click actions under New File"), isOn: Binding(
                    get: { store.configuration.useSubmenu },
                    set: { enabled in store.update { $0.useSubmenu = enabled } }
                ))
            }
            Section(L("File Types")) {
                Text(L("Enable types, edit starter templates, and reorder them. The first enabled type is the toolbar’s default action. Leave the filename empty for dotfiles."))
                    .font(.caption).foregroundStyle(.secondary)
                ForEach(Array(store.configuration.types.enumerated()), id: \.element.id) { index, type in
                    HStack {
                        Toggle(type.title, isOn: Binding(get: {
                            store.configuration.types.first(where: { $0.id == type.id })?.enabled ?? false
                        }, set: { enabled in
                            guard let i = store.configuration.types.firstIndex(where: { $0.id == type.id }) else { return }
                            store.update { $0.types[i].enabled = enabled }
                        }))
                        Spacer()
                        Text(".\(type.fileExtension)").foregroundStyle(.secondary)
                        if allowsCreation {
                            Button(L("Create…")) { store.create(type) }.disabled(!type.enabled)
                        }
                        Button { move(index, by: -1) } label: { Image(systemName: "arrow.up") }
                            .disabled(index == 0).help(L("Move Up"))
                        Button { move(index, by: 1) } label: { Image(systemName: "arrow.down") }
                            .disabled(index == store.configuration.types.count - 1).help(L("Move Down"))
                        Button(L("Edit…")) { editing = type }
                        Button(role: .destructive) {
                            store.update { $0.types.removeAll { $0.id == type.id } }
                        } label: { Image(systemName: "trash") }.help(L("Delete file type"))
                    }
                    .draggable(type.id)
                    .dropDestination(for: String.self) { identifiers, _ in
                        guard let id = identifiers.first,
                              let source = store.configuration.types.firstIndex(where: { $0.id == id }),
                              let target = store.configuration.types.firstIndex(where: { $0.id == type.id }),
                              source != target else { return false }
                        return store.update {
                            let moved = $0.types.remove(at: source)
                            $0.types.insert(moved, at: target)
                        }
                    }
                }
                Button(L("Add File Type…")) {
                    editing = NewFileType(fileExtension: "", baseName: "Untitled", menuTitle: "", enabled: true)
                }
            }
            if let error = store.error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
        }
        .formStyle(.grouped)
        .onAppear {
            extensionEnabled = FIFinderSyncController.isExtensionEnabled
            showExtensionWarning = !extensionEnabled
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            let previouslyEnabled = extensionEnabled
            extensionEnabled = FIFinderSyncController.isExtensionEnabled
            if extensionEnabled { showExtensionWarning = false }
            else if previouslyEnabled { showExtensionWarning = true }
        }
        .alert(L("Enable New File in Finder"), isPresented: $showExtensionWarning) {
            Button(L("Open Extension Settings")) { FIFinderSyncController.showExtensionManagementInterface() }
            Button(L("Not Now"), role: .cancel) {}
        } message: {
            Text(extensionSetupInstructions)
        }
        .sheet(item: $editing) { type in
            NewFileTypeEditor(type: type) { updated in
                let previous = store.configuration
                if let index = store.configuration.types.firstIndex(where: { $0.id == updated.id }) {
                    store.configuration.types[index] = updated
                } else { store.configuration.types.append(updated) }
                guard store.save() else { store.configuration = previous; return false }
                return true
            }
        }
    }

    private func move(_ index: Int, by offset: Int) {
        store.update { $0.types.swapAt(index, index + offset) }
    }
}

private struct NewFileTypeEditor: View {
    @State var type: NewFileType
    var onSave: (NewFileType) -> Bool
    @State private var error: String?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L("File Type")).font(.headline)
            TextField(L("Extension (e.g. txt or tsx)"), text: $type.fileExtension)
            TextField(L("Menu label"), text: $type.menuTitle)
            TextField(L("Default filename (empty for dotfiles)"), text: $type.baseName)
            Text(L("Starter template"))
            TextEditor(text: $type.template).font(.system(.body, design: .monospaced))
                .frame(minHeight: 180).border(.secondary.opacity(0.3))
            if let error { Text(error).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button(L("Cancel")) { dismiss() }.keyboardShortcut(.cancelAction)
                Button(L("Save")) {
                    do {
                        let validated = try type.validated()
                        if onSave(validated) { dismiss() }
                        else { error = L("Could not save the file type. Check that Kebloop’s settings folder is writable.") }
                    } catch { self.error = error.localizedDescription }
                }.keyboardShortcut(.defaultAction)
            }
        }.padding(20).frame(width: 520)
    }
}
