import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct RaycastMigrationView: View {
    @ObservedObject var snippets: SnippetStore
    @ObservedObject var quicklinks: QuicklinkStore
    @State private var items: [RaycastImportItem] = []
    @State private var selected: Set<String> = []
    @State private var message = ""
    @State private var includeSnippets = true
    @State private var includeQuicklinks = true
    private var visible: [RaycastImportItem] { items.filter { $0.kind == .snippet ? includeSnippets : includeQuicklinks } }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L("Import from Raycast")).font(.title2.bold())
            Text(L("In Raycast, run Export Snippets or Export Quicklinks and choose the JSON here. Encrypted .rayconfig backups are not supported.")).foregroundStyle(.secondary)
            Text(L("Existing items are preserved. Snippet keywords are omitted so import cannot activate typing triggers. Dynamic snippet placeholders must be converted manually.")).font(.callout)
            HStack {
                Button(L("Choose JSON…"), action: chooseFile)
                Toggle(L("Snippets"), isOn: $includeSnippets)
                Toggle(L("Quicklinks"), isOn: $includeQuicklinks)
            }
            List(visible) { item in
                let issue = RaycastMigration.conflict(item, snippets: snippets, quicklinks: quicklinks)
                HStack(alignment: .top) {
                    Toggle("Import \(item.name)", isOn: Binding(get: { selected.contains(item.id) }, set: { if $0 { selected.insert(item.id) } else { selected.remove(item.id) } })).labelsHidden().disabled(issue != nil)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("\(item.kind.rawValue): \(item.name)").bold()
                        Text(item.value).font(.caption).lineLimit(3).textSelection(.enabled)
                        if let issue { Text(issue).font(.caption).foregroundStyle(.orange) }
                        if item.kind == .snippet && !item.keyword.isEmpty { Text("Keyword \(item.keyword) will be omitted; add it manually later.").font(.caption).foregroundStyle(.secondary) }
                    }
                }
            }
            HStack {
                Button(L("Select valid items")) { selected = Set(visible.filter { RaycastMigration.conflict($0, snippets: snippets, quicklinks: quicklinks) == nil }.map(\.id)) }
                Button(L("Import selected")) {
                    do {
                        let result = try RaycastMigration.importItems(visible, selected: selected, snippets: snippets, quicklinks: quicklinks)
                        message = "Imported \(result.imported) items. Skipped \(result.skipped) conflicts."
                        selected = []
                    } catch { message = error.localizedDescription }
                }.disabled(!visible.contains { selected.contains($0.id) })
            }
            if !message.isEmpty { Text(message).font(.callout).textSelection(.enabled) }
        }.padding(20).frame(minWidth: 620, minHeight: 480)
    }
    private func chooseFile() {
        let panel = NSOpenPanel(); panel.allowedContentTypes = [.json]; panel.canChooseDirectories = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            guard url.pathExtension.lowercased() != "rayconfig" else { throw ConfigurationError.invalid("Encrypted .rayconfig backups are unsupported. Export Snippets or Quicklinks as JSON in Raycast.") }
            items = try RaycastMigration.preview(data: Data(contentsOf: url))
            selected = []; message = L("Preview loaded. Choose the items to import.")
        } catch { items = []; selected = []; message = error.localizedDescription }
    }
}

struct RaycastCompatibleCommandsView: View {
    @ObservedObject var scripts: ScriptCommandStore
    @ObservedObject var sdk: RaycastSDKStore
    @State private var candidate: ScriptCommand?
    @State private var message = ""
    var body: some View {
        ScrollView { VStack(alignment: .leading, spacing: 14) {
            RaycastSDKCompatibilityView(store: sdk)
            Divider()
            Text(L("Raycast Script Commands")).font(.title2.bold())
            Text(L("Import existing Raycast Script Commands with schema version 1 and silent, compact or fullOutput modes. Kebloop runs their original interpreter and shows plain text output in its script runner."))
            Text(L("React extensions using @raycast/api, inline refresh, argument forms, custom working-directory metadata and scripts requiring per-run confirmation are unsupported. These are rejected rather than executed.")).foregroundStyle(.secondary)
            Text(L("Scripts run with your account's access, without a sandbox. Review the source and any API credentials or paid service calls before enabling. Import never executes code or reads Raycast credentials.")).font(.callout)
            Button(L("Choose Raycast script…")) {
                let panel = NSOpenPanel(); panel.canChooseDirectories = false
                guard panel.runModal() == .OK, let url = panel.url else { return }
                do { candidate = try RaycastScriptCompatibility.inspect(url: url); message = L("Review the source before importing.") }
                catch { candidate = nil; message = error.localizedDescription }
            }
            if let command = candidate {
                Text(command.title).font(.headline)
                Text(command.description)
                Text(command.arguments.last ?? "").font(.caption.monospaced()).textSelection(.enabled)
                HStack {
                    Button(L("Review source")) { if let path = command.arguments.last { NSWorkspace.shared.open(URL(fileURLWithPath: path)) } }
                    Button(L("Import disabled command")) {
                        do { try scripts.importCommands([command]); candidate = nil; message = L("Imported disabled. Enable it explicitly in Script Commands after review.") }
                        catch { message = error.localizedDescription }
                    }
                }
            }
            if !message.isEmpty { Text(message).foregroundStyle(.secondary).textSelection(.enabled) }
            Spacer()
        }.padding(20) }.frame(minWidth: 650, minHeight: 580)
    }
}
