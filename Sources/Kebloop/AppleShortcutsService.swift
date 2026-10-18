import Foundation
import Combine

struct AppleShortcut: Codable, Identifiable, Hashable {
    let id: String
    var name: String
}

@MainActor
final class AppleShortcutsService: ObservableObject {
    @Published private(set) var available: [AppleShortcut] = []
    @Published private(set) var selected: [AppleShortcut] = []
    @Published private(set) var isLoading = false
    @Published private(set) var isRunning = false
    @Published private(set) var output: String?
    @Published private(set) var runningID: String?
    @Published private(set) var errors: [String: String] = [:]
    @Published private(set) var error: String?
    private let defaults: UserDefaults
    private let executableURL: URL
    private var process: ShortcutProcess?
    private var completion: ((String?) -> Void)?
    private var generation = UUID()
    private var missingIDs = Set<String>()
    init(defaults: UserDefaults = .standard, executableURL: URL = URL(fileURLWithPath: "/usr/bin/shortcuts")) { self.defaults = defaults; self.executableURL = executableURL; reload() }
    func reload() {
        cancel()
        missingIDs = []; errors = [:]
        selected = defaults.data(forKey: "selectedAppleShortcuts").flatMap { try? JSONDecoder().decode([AppleShortcut].self, from: $0) } ?? []
    }
    static func parseList(_ text: String) -> [AppleShortcut] {
        text.split(whereSeparator: \.isNewline).compactMap { line in
            guard let open = line.range(of: " (", options: .backwards), line.hasSuffix(")") else { return nil }
            let id = String(line[open.upperBound..<line.index(before: line.endIndex)])
            guard UUID(uuidString: id) != nil else { return nil }
            return AppleShortcut(id: id, name: String(line[..<open.lowerBound]))
        }
    }
    func refresh() {
        guard !isLoading else { return }
        isLoading = true; error = nil
        Task {
            let executableURL = self.executableURL
            let result = await Task.detached { Self.execute(arguments: ["list", "--show-identifiers"], executableURL: executableURL) }.value
            isLoading = false
            switch result {
            case .success(let output):
                available = Self.parseList(output)
                missingIDs = Set(selected.map(\.id)).subtracting(available.map(\.id))
                for index in selected.indices {
                    if let shortcut = available.first(where: { $0.id == selected[index].id }) { selected[index].name = shortcut.name; errors[shortcut.id] = nil }
                    else { errors[selected[index].id] = "Shortcut missing on this Mac. Refresh or select a replacement." }
                }
                persist()
            case .failure(let failure): error = failure.localizedDescription
            }
        }
    }
    func select(_ shortcut: AppleShortcut) {
        guard !selected.contains(where: { $0.id == shortcut.id }) else { return }
        selected.append(shortcut); persist()
    }
    func remove(id: String) {
        if runningID == id { cancel() }
        selected.removeAll { $0.id == id }; errors[id] = nil; missingIDs.remove(id); persist()
    }
    func run(id: String, input: String? = nil, completion: ((String?) -> Void)? = nil) {
        guard !isRunning else { completion?("Another shortcut is already running."); return }
        guard selected.contains(where: { $0.id == id }) else { completion?("This shortcut is not selected."); return }
        if missingIDs.contains(id) { completion?(errors[id]); return }
        output = nil
        let token = UUID(); generation = token
        let taskProcess = ShortcutProcess(); process = taskProcess
        self.completion = completion
        runningID = id; isRunning = true; errors[id] = nil
        Task {
            let executableURL = self.executableURL
            let result = await Task.detached { Self.execute(arguments: ["run", id], input: input, process: taskProcess, executableURL: executableURL) }.value
            guard generation == token else { return }
            process = nil; runningID = nil; isRunning = false; self.completion = nil
            switch result {
            case .success(let resultOutput):
                output = resultOutput
                completion?(nil)
            case .failure(let failure): errors[id] = failure.localizedDescription; completion?(failure.localizedDescription)
            }
        }
    }
    func cancel() {
        generation = UUID()
        process?.cancel()
        let callback = completion; completion = nil
        process = nil; isRunning = false; runningID = nil
        callback?("Shortcut cancelled.")
    }
    private func persist() {
        if let data = try? JSONEncoder().encode(selected) { defaults.set(data, forKey: "selectedAppleShortcuts") }
    }
    // CLI execution and output draining happen away from the main actor.
    nonisolated private static func execute(arguments: [String], input: String? = nil, process: ShortcutProcess = ShortcutProcess(), executableURL: URL) -> Result<String, Error> {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("kebloop-shortcut-\(UUID().uuidString)", isDirectory: true)
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: directory) }
            var arguments = arguments
            if let input {
                let inputURL = directory.appendingPathComponent("input.txt")
                try input.write(to: inputURL, atomically: true, encoding: .utf8)
                arguments += ["--input-path", inputURL.path]
            }
            let outputURL = directory.appendingPathComponent("output.txt")
            _ = FileManager.default.createFile(atPath: outputURL.path, contents: nil)
            let output = try FileHandle(forWritingTo: outputURL)
            defer { try? output.close() }
            process.process.executableURL = executableURL
            process.process.arguments = arguments
            process.process.standardOutput = output; process.process.standardError = output
            try process.start(); process.process.waitUntilExit()
            let text = String(data: try Data(contentsOf: outputURL), encoding: .utf8) ?? ""
            guard process.process.terminationStatus == 0 else {
                return .failure(NSError(domain: "AppleShortcuts", code: Int(process.process.terminationStatus), userInfo: [NSLocalizedDescriptionKey: text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Shortcut failed. Check Shortcuts permissions and availability." : text.trimmingCharacters(in: .whitespacesAndNewlines)]))
            }
            return .success(text)
        } catch { return .failure(error) }
    }
}

// Cancellation can arrive before the background task launches the process.
private final class ShortcutProcess: @unchecked Sendable {
    let process = Process()
    private let lock = NSLock()
    private var cancelled = false
    func start() throws {
        lock.lock(); defer { lock.unlock() }
        guard !cancelled else { throw CancellationError() }
        try process.run()
    }
    func cancel() {
        lock.lock(); defer { lock.unlock() }
        cancelled = true
        if process.isRunning { process.terminate() }
    }
}
