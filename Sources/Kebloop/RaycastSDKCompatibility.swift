import AppKit
import Combine
import Foundation

struct RaycastSDKPreference: Codable, Identifiable {
    let name: String
    let title: String?
    let type: String
    let required: Bool?
    let `default`: String?
    var id: String { name }
}
struct RaycastSDKCommand: Codable, Identifiable {
    var id: String = UUID().uuidString
    let title: String
    let source: String
    let runtime: String
    let preferences: [RaycastSDKPreference]
    var values: [String: String] = [:]
    var enabled = false
}

enum RaycastSDKCompatibility {
    private struct Manifest: Decodable {
        struct Command: Decodable { struct Argument: Decodable { let name: String? }; let name: String; let title: String; let mode: String; let preferences: [RaycastSDKPreference]?; let arguments: [Argument]? }
        let commands: [Command]
        let preferences: [RaycastSDKPreference]?
    }
    struct Effect: Decodable { let type: String; let value: String }
    struct Response: Decodable { let effects: [Effect]; let error: String? }
    static func inspect(manifestURL: URL, commandName: String, entryURL: URL, runtime: String) throws -> RaycastSDKCommand {
        guard ["node", "bun"].contains(URL(fileURLWithPath: runtime).lastPathComponent), runtime.hasPrefix("/"), FileManager.default.isExecutableFile(atPath: runtime) else { throw AutomationFailure.invalid("Choose an installed node or bun executable by absolute path.") }
        let manifestData = try Data(contentsOf: manifestURL)
        guard manifestData.count <= 1_000_000 else { throw AutomationFailure.invalid("The package manifest exceeds 1 MB.") }
        let manifest = try JSONDecoder().decode(Manifest.self, from: manifestData)
        guard let definition = manifest.commands.first(where: { $0.name == commandName }), definition.mode == "no-view" else { throw AutomationFailure.invalid("Select a manifest command whose mode is no-view. React view commands are unsupported.") }
        guard definition.arguments?.isEmpty != false else { throw AutomationFailure.invalid("Command argument forms are unsupported in this runtime.") }
        let preferences = (manifest.preferences ?? []) + (definition.preferences ?? [])
        guard Set(preferences.map(\.name)).count == preferences.count, preferences.allSatisfy({ ["textfield", "textarea"].contains($0.type) && !$0.name.isEmpty }) else { throw AutomationFailure.invalid("Only unique textfield and textarea preferences are supported. Password, dropdown, OAuth and other preference types are unsupported.") }
        guard ["js", "mjs"].contains(entryURL.pathExtension.lowercased()) else { throw AutomationFailure.invalid("Choose a compiled, standalone .js or .mjs entry. TypeScript and JSX require compilation outside Kebloop.") }
        let source = try Data(contentsOf: entryURL)
        guard source.count <= 1_000_000, String(data: source, encoding: .utf8) != nil else { throw AutomationFailure.invalid("Choose a UTF-8 JavaScript command under 1 MB.") }
        return RaycastSDKCommand(title: definition.title, source: entryURL.standardizedFileURL.path, runtime: runtime, preferences: preferences)
    }

    static func execute(_ command: RaycastSDKCommand, runner: AutomationProcess) throws -> Response {
        guard command.enabled else { throw AutomationFailure.invalid("Review and enable the imported JavaScript extension before running it.") }
        guard command.source.hasPrefix("/"), !command.source.contains("\0"),
              command.runtime.hasPrefix("/"), !command.runtime.contains("\0"),
              ["node", "bun"].contains(URL(fileURLWithPath: command.runtime).lastPathComponent),
              FileManager.default.isExecutableFile(atPath: command.runtime),
              !command.id.isEmpty, command.title.utf8.count <= 200,
              command.preferences.count <= 100,
              Set(command.preferences.map(\.name)).count == command.preferences.count,
              command.preferences.allSatisfy({ !$0.name.isEmpty && $0.name.utf8.count <= 200 && ["textfield", "textarea"].contains($0.type) && ($0.default?.utf8.count ?? 0) <= 10_000 }),
              command.values.count <= 100, command.values.allSatisfy({ $0.key.utf8.count <= 200 && $0.value.utf8.count <= 10_000 }) else { throw AutomationFailure.invalid("Invalid stored Raycast extension configuration. Reimport after reviewing the manifest and runtime.") }
        guard command.preferences.allSatisfy({ $0.required != true || !(command.values[$0.name] ?? $0.default ?? "").isEmpty }) else { throw AutomationFailure.invalid("Fill all required extension preferences before running.") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("kebloop-raycast-sdk-\(UUID())", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let package = directory.appendingPathComponent("node_modules/@raycast/api", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try FileManager.default.createDirectory(at: package, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let source = try Data(contentsOf: URL(fileURLWithPath: command.source))
        guard source.count <= 1_000_000 else { throw AutomationFailure.invalid("The command now exceeds 1 MB.") }
        try source.write(to: directory.appendingPathComponent("command.mjs"))
        try #"{"type":"module","exports":"./index.mjs"}"#.write(to: package.appendingPathComponent("package.json"), atomically: true, encoding: .utf8)
        try shim.write(to: package.appendingPathComponent("index.mjs"), atomically: true, encoding: .utf8)
        try host.write(to: directory.appendingPathComponent("host.mjs"), atomically: true, encoding: .utf8)
        let values = Dictionary(uniqueKeysWithValues: command.preferences.map { ($0.name, command.values[$0.name] ?? $0.default ?? "") })
        let output = try runner.execute(executable: command.runtime, arguments: [directory.appendingPathComponent("host.mjs").path], input: JSONEncoder().encode(values), timeout: 30)
        let response = try JSONDecoder().decode(Response.self, from: Data(output.stdout.utf8))
        if let error = response.error { throw AutomationFailure.invalid(error) }
        guard response.effects.count <= 100, response.effects.allSatisfy({ ["copy", "message", "open"].contains($0.type) && $0.value.utf8.count <= 100_000 }) else { throw AutomationFailure.invalid("The extension returned invalid effects.") }
        for effect in response.effects where effect.type == "open" {
            guard let url = Quicklink.destination(effect.value), !url.isFileURL else { throw AutomationFailure.invalid("Extension open only supports HTTP(S) URLs.") }
        }
        return response
    }

    private static let shim = #"""
    const effects = globalThis.__kebloopEffects;
    function text(value) { if (typeof value !== "string" && typeof value !== "number") throw new Error("Only plain text is supported"); return String(value); }
    function emit(type, value) { if (effects.length >= 100 || value.length > 100000) throw new Error("Effect limit exceeded"); effects.push({ type, value }); }
    export const Clipboard = Object.freeze({ copy: async (value, options) => { if (options && Object.keys(options).length) throw new Error("Clipboard options are unsupported"); emit("copy", text(value)); } });
    export const Toast = Object.freeze({ Style: Object.freeze({ Success: "success", Failure: "failure", Animated: "animated" }) });
    export async function showToast(options) { if (typeof options !== "object" || options.primaryAction || options.secondaryAction) throw new Error("Toast actions are unsupported"); emit("message", text(options.title) + (options.message ? ": " + text(options.message) : "")); return { hide: async () => { throw new Error("Toast mutation/hiding is unsupported"); } }; }
    export async function showHUD(value) { emit("message", text(value)); }
    export function getPreferenceValues() { return { ...globalThis.__kebloopPreferences }; }
    export async function open(value, application) { if (application) throw new Error("Application selection is unsupported"); const url = new URL(text(value)); if (!["http:", "https:"].includes(url.protocol)) throw new Error("Only HTTP(S) URLs are supported"); emit("open", url.href); }
    """#
    private static let host = #"""
    import { readFileSync } from "node:fs";
    globalThis.__kebloopEffects = [];
    globalThis.__kebloopPreferences = JSON.parse(readFileSync(0, "utf8"));
    console.log = console.info = (...args) => console.error(...args);
    try {
      const command = await import("./command.mjs");
      if (typeof command.default !== "function") throw new Error("The no-view command must export a default function");
      await command.default({ arguments: {}, launchContext: undefined });
      process.stdout.write(JSON.stringify({ effects: globalThis.__kebloopEffects }));
    } catch (error) { process.stdout.write(JSON.stringify({ effects: [], error: "Unsupported API or extension failure: " + error.message })); }
    """#
}

@MainActor final class RaycastSDKStore: ObservableObject {
    @Published private(set) var commands: [RaycastSDKCommand] = []
    @Published private(set) var running = false
    @Published private(set) var message = ""
    private let defaults: UserDefaults
    private var runner: AutomationProcess?
    private var generation = UUID()
    private var activeID: String?
    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        commands = defaults.data(forKey: "raycastSDKCommands").flatMap { try? JSONDecoder().decode([RaycastSDKCommand].self, from: $0) } ?? []
    }
    func install(_ command: RaycastSDKCommand) {
        var command = command; command.enabled = false
        commands.append(command); persist()
    }
    func enable(_ id: String, _ value: Bool) { if !value && activeID == id { cancel() }; guard let index = commands.firstIndex(where: { $0.id == id }) else { return }; commands[index].enabled = value; persist() }
    func remove(_ id: String) { if activeID == id { cancel() }; commands.removeAll { $0.id == id }; persist() }
    func setValue(_ id: String, name: String, value: String) { guard let index = commands.firstIndex(where: { $0.id == id }), commands[index].preferences.contains(where: { $0.name == name }) else { return }; commands[index].values[name] = value; persist() }
    func value(_ id: String, preference: RaycastSDKPreference) -> String { commands.first(where: { $0.id == id })?.values[preference.name] ?? preference.default ?? "" }
    func run(_ id: String) {
        guard !running, let command = commands.first(where: { $0.id == id && $0.enabled }) else { return }
        let process = AutomationProcess(), token = UUID(); generation = token; activeID = id; runner = process; running = true; message = L("Running…")
        Task {
            let result = await Task.detached { Result { try RaycastSDKCompatibility.execute(command, runner: process) } }.value
            guard generation == token else { return }
            running = false; runner = nil; activeID = nil
            switch result {
            case .failure(let error): message = error.localizedDescription
            case .success(let response):
                var messages: [String] = []
                for effect in response.effects {
                    switch effect.type {
                    case "copy": NSPasteboard.general.clearContents(); NSPasteboard.general.setString(effect.value, forType: .string); messages.append(L("Copied text."))
                    case "open": if let url = URL(string: effect.value) { if !NSWorkspace.shared.open(url) { messages.append(L("Could not open URL.")) } }
                    default: messages.append(effect.value)
                    }
                }
                message = messages.isEmpty ? L("Command finished.") : messages.joined(separator: "\n")
            }
        }
    }
    func cancel() { generation = UUID(); runner?.cancel(); runner = nil; activeID = nil; running = false; message = L("Cancelled.") }
    private func persist() { if let data = try? JSONEncoder().encode(commands) { defaults.set(data, forKey: "raycastSDKCommands") } }
}
