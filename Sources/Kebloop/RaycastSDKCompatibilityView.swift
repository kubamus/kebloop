import AppKit
import SwiftUI

struct RaycastSDKCompatibilityView: View {
    @ObservedObject var store: RaycastSDKStore
    @State private var manifestPath = ""
    @State private var commandName = ""
    @State private var entryPath = ""
    @State private var runtimePath = ""
    @State private var candidate: RaycastSDKCommand?
    @State private var error = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(L("Raycast JavaScript extensions")).font(.headline)
            Text(L("Runs compiled standalone no-view commands importing @raycast/api. Supported: Clipboard.copy (plain text), showToast/showHUD messages, getPreferenceValues and open (HTTP(S)). React UI, OAuth, AI, clipboard reads/paste, dependencies and other APIs are unsupported.")).font(.caption)
            Text(L("Code runs with your account's filesystem and network access. Review source and paid API calls first. Nonsecret text preferences are stored locally. Password preferences and credential migration are rejected.")).font(.caption).foregroundStyle(.secondary)
            pathField(L("Raycast package.json"), value: $manifestPath)
            TextField(L("Command name from manifest"), text: $commandName)
            pathField(L("Compiled .js / .mjs entry"), value: $entryPath)
            pathField(L("Installed node or bun executable"), value: $runtimePath)
            Button(L("Validate and preview")) {
                do { candidate = try RaycastSDKCompatibility.inspect(manifestURL: URL(fileURLWithPath: manifestPath), commandName: commandName, entryURL: URL(fileURLWithPath: entryPath), runtime: runtimePath); error = "" }
                catch { candidate = nil; self.error = error.localizedDescription }
            }
            if let candidate {
                HStack {
                    Text(candidate.title).bold()
                    Button(L("Review JavaScript")) { NSWorkspace.shared.open(URL(fileURLWithPath: candidate.source)) }
                    Button(L("Import disabled extension")) { store.install(candidate); self.candidate = nil }
                }
            }
            if !error.isEmpty { Text(error).font(.caption).foregroundStyle(.orange) }
            ForEach(store.commands) { command in
                VStack(alignment: .leading) {
                    HStack {
                        Toggle(command.title, isOn: Binding(get: { command.enabled }, set: { store.enable(command.id, $0) })).disabled(store.running)
                        Button(L("Run")) { store.run(command.id) }.disabled(!command.enabled || store.running)
                        Button(L("Review")) { NSWorkspace.shared.open(URL(fileURLWithPath: command.source)) }
                        Button(L("Remove")) { store.remove(command.id) }.disabled(store.running)
                    }
                    ForEach(command.preferences) { preference in
                        TextField(preference.title ?? preference.name, text: Binding(get: { store.value(command.id, preference: preference) }, set: { store.setValue(command.id, name: preference.name, value: $0) }))
                    }
                }
            }
            if store.running { Button(L("Cancel"), action: store.cancel) }
            if !store.message.isEmpty { Text(store.message).font(.caption).textSelection(.enabled) }
        }.onDisappear { store.cancel() }
    }
    private func pathField(_ title: String, value: Binding<String>) -> some View {
        HStack {
            TextField(title, text: value)
            Button(L("Choose…")) {
                let panel = NSOpenPanel(); panel.canChooseDirectories = false
                if panel.runModal() == .OK, let url = panel.url { value.wrappedValue = url.path; candidate = nil }
            }
        }
    }
}
