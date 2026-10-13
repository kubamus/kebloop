import Foundation

/// Adapts real @raycast metadata without modifying or evaluating the script at import time.
enum RaycastScriptCompatibility {
    static func inspect(url: URL) throws -> ScriptCommand {
        let data = try Data(contentsOf: url)
        guard data.count <= 1_000_000, let source = String(data: data, encoding: .utf8) else { throw AutomationFailure.invalid("Choose a UTF-8 Raycast script under 1 MB.") }
        return try inspect(source: source, path: url.standardizedFileURL.path)
    }
    static func inspect(source: String, path: String) throws -> ScriptCommand {
        var metadata: [String: String] = [:]
        for line in source.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard trimmed.hasPrefix("#") || trimmed.hasPrefix("//"), let range = trimmed.range(of: "@raycast.") else { continue }
            let fields = trimmed[range.upperBound...].split(maxSplits: 1, whereSeparator: { $0.isWhitespace })
            guard fields.count == 2 else { continue }
            let key = String(fields[0])
            guard metadata[key] == nil else { throw AutomationFailure.invalid("Duplicate Raycast metadata: \(key).") }
            metadata[key] = String(fields[1]).trimmingCharacters(in: .whitespaces)
        }
        guard metadata["schemaVersion"] == "1", let title = metadata["title"], !title.isEmpty else { throw AutomationFailure.invalid("Expected @raycast.schemaVersion 1 and @raycast.title.") }
        guard let mode = metadata["mode"], ["silent", "compact", "fullOutput"].contains(mode) else { throw AutomationFailure.invalid("Only silent, compact and fullOutput scripts are supported. Inline refresh is not available.") }
        for key in metadata.keys where key.hasPrefix("argument") || ["currentDirectoryPath", "refreshTime"].contains(key) {
            throw AutomationFailure.invalid("Raycast \(key) metadata is not supported; adapt this script explicitly first.")
        }
        guard metadata["needsConfirmation"] != "true" else { throw AutomationFailure.invalid("Scripts requiring per-run confirmation are not supported.") }
        guard metadata["platform"] == nil || metadata["platform"] == "macos" else { throw AutomationFailure.invalid("Only macOS scripts are supported.") }
        guard let first = source.components(separatedBy: .newlines).first, first.hasPrefix("#!/") else { throw AutomationFailure.invalid("The script must have an absolute interpreter shebang.") }
        let interpreter = first.dropFirst(2).split(whereSeparator: { $0.isWhitespace }).map(String.init)
        guard let executable = interpreter.first, !executable.contains("\0"), !path.contains("\0"), path.hasPrefix("/") else { throw AutomationFailure.invalid("Invalid interpreter or script path.") }
        // Resolve cwd-dependent scripts using an explicit wrapper: Process otherwise inherits Kebloop's cwd.
        let command = ScriptCommand(id: UUID().uuidString, title: title, description: "Raycast \(mode) · " + (metadata["description"] ?? "Imported script; review before enabling."), executable: "/bin/sh", arguments: ["-c", "cd -- \"$1\" && shift && exec \"$@\"", "kebloop-raycast", URL(fileURLWithPath: path).deletingLastPathComponent().path] + interpreter + [path], enabled: false)
        try command.validate()
        return command
    }
}
