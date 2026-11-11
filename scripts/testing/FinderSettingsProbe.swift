import Foundation

/// Compiled twice by test-finder-settings.sh: once as an unsandboxed host, once
/// with the Finder extension's actual sandbox entitlements and bundle identity.
@main
enum FinderSettingsProbe {
    private struct Snapshot: Encodable {
        let path: String
        let configuration: NewFileConfiguration
    }

    static func main() throws {
        if let index = CommandLine.arguments.firstIndex(of: "--test-menu"),
           CommandLine.arguments.indices.contains(index + 1) {
            try FinderMenuChecks.run(in: URL(fileURLWithPath: CommandLine.arguments[index + 1], isDirectory: true))
            return
        }
        let url = try NewFileConfiguration.storageURL
        if CommandLine.arguments.contains("--expect-sandbox") {
            // This must be a real sandbox test; a normal process cannot reproduce
            // Foundation's home-directory redirection.
            guard NSHomeDirectory().contains("/Library/Containers/"),
                  !url.path.hasPrefix(NSHomeDirectory() + "/") else {
                throw NSError(domain: "FinderSettingsProbe", code: 1, userInfo: [
                    NSLocalizedDescriptionKey: "Expected a sandboxed process reading settings outside its container."
                ])
            }
        }
        let snapshot = Snapshot(path: url.path, configuration: try NewFileConfiguration.load())
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        FileHandle.standardOutput.write(try encoder.encode(snapshot))
    }
}
