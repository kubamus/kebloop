import XCTest
@testable import Kebloop

final class RaycastMigrationTests: XCTestCase {
    func testRealJSONFormatAndUnsupportedPlaceholders() throws {
        let data = Data(#"[{"name":"Email","text":"hello@example.com","keyword":"!email"},{"name":"Dynamic","text":"Hi {cursor}"},{"name":"Search","link":"https://example.com/?q={query}","keyword":"find"},{"name":"Unsafe","link":"javascript:alert(1)"}]"#.utf8)
        let items = try RaycastMigration.preview(data: data)
        XCTAssertEqual(items.count, 4)
        XCTAssertNil(items[0].issue)
        XCTAssertEqual(items[0].keyword, "!email")
        XCTAssertNotNil(items[1].issue)
        XCTAssertNil(items[2].issue)
        XCTAssertNotNil(items[3].issue)
        XCTAssertThrowsError(try RaycastMigration.preview(data: Data("encrypted-rayconfig".utf8)))
    }

    @MainActor func testSelectiveAdditiveImportPreservesConflictsAndOmitsTypingTriggers() throws {
        let suite = "RaycastMigration.\(UUID())", defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let snippets = SnippetStore(defaults: defaults), links = QuicklinkStore(defaults: defaults)
        try snippets.save(name: "Existing", template: "keep", keyword: "!keep")
        XCTAssertNil(links.save(name: "Existing link", target: "https://existing.example", keyword: "taken"))
        let items = try RaycastMigration.preview(data: Data(#"[{"name":" Existing ","text":"replace"},{"name":"New","text":"new","keyword":"!new"},{"name":"Unselected","text":"skip"},{"name":"Conflicting link","link":"https://other.example","keyword":"taken"}]"#.utf8))
        let result = try RaycastMigration.importItems(items, selected: ["0", "1", "3"], snippets: snippets, quicklinks: links)
        XCTAssertEqual(result.imported, 1); XCTAssertEqual(result.skipped, 2)
        XCTAssertEqual(snippets.snippets.count, 2)
        XCTAssertEqual(snippets.snippets[0].template, "keep")
        XCTAssertEqual(snippets.snippets[0].keyword, "!keep")
        XCTAssertNil(snippets.snippets[1].keyword)
        XCTAssertEqual(links.links.count, 1)
        XCTAssertEqual(SnippetStore(defaults: defaults).snippets, snippets.snippets)
    }

    func testDuplicateItemsWithinBatchAreSkipped() async throws {
        try await MainActor.run {
            let suite = "RaycastDuplicate.\(UUID())", defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
            defer { defaults.removePersistentDomain(forName: suite) }
            let items = try RaycastMigration.preview(data: Data(#"[{"name":"A","text":"same"},{"name":"B","text":"same"}]"#.utf8))
            let result = try RaycastMigration.importItems(items, selected: ["0", "1"], snippets: SnippetStore(defaults: defaults), quicklinks: QuicklinkStore(defaults: defaults))
            XCTAssertEqual(result.imported, 1); XCTAssertEqual(result.skipped, 1)
        }
    }

    @MainActor func testInvalidBatchWritesNothing() throws {
        let suite = "RaycastAtomic.\(UUID())", defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let snippets = SnippetStore(defaults: defaults), links = QuicklinkStore(defaults: defaults)
        let items = [RaycastImportItem(id: "1", kind: .snippet, name: "Valid", value: "text", keyword: "", issue: nil), RaycastImportItem(id: "2", kind: .quicklink, name: "Invalid", value: "javascript:alert(1)", keyword: "", issue: nil)]
        XCTAssertThrowsError(try RaycastMigration.importItems(items, selected: ["1", "2"], snippets: snippets, quicklinks: links))
        XCTAssertTrue(snippets.snippets.isEmpty); XCTAssertTrue(links.links.isEmpty)
    }

    func testActualRaycastScriptRunsWithLiteralPathAndWorkingDirectory() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("Raycast ' $(noop) \(UUID())", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("script ' $.sh")
        let source = "#!/bin/sh\n# @raycast.schemaVersion 1\n# @raycast.title Greeting\n# @raycast.mode fullOutput\nprintf '%s' \"$PWD\"\n"
        try source.write(to: url, atomically: true, encoding: .utf8)
        let command = try RaycastScriptCompatibility.inspect(url: url)
        XCTAssertFalse(command.enabled)
        let output = try AutomationProcess().execute(executable: command.executable, arguments: command.arguments)
        XCTAssertEqual(URL(fileURLWithPath: output.stdout).standardizedFileURL.resolvingSymlinksInPath(), directory.standardizedFileURL.resolvingSymlinksInPath())
        XCTAssertTrue(FileManager.default.fileExists(atPath: url.path))
    }

    func testUnsupportedRaycastFeaturesAreRejectedBeforeExecution() {
        let source = "#!/bin/sh\n# @raycast.schemaVersion 1\n# @raycast.title Test\n# @raycast.mode silent\n"
        for metadata in ["# @raycast.argument1 {\"type\":\"password\"}", "# @raycast.needsConfirmation true", "# @raycast.currentDirectoryPath /tmp", "# @raycast.platform windows", "# @raycast.title Duplicate"] {
            XCTAssertThrowsError(try RaycastScriptCompatibility.inspect(source: source + metadata, path: "/tmp/test.sh"))
        }
        XCTAssertThrowsError(try RaycastScriptCompatibility.inspect(source: source.replacingOccurrences(of: "silent", with: "inline"), path: "/tmp/test.sh"))
    }
}

final class RaycastSDKCompatibilityTests: XCTestCase {
    private func installedRuntime() throws -> String {
        for path in ["/opt/homebrew/bin/bun", "/usr/local/bin/node", "/opt/homebrew/bin/node", "/usr/bin/node"] where FileManager.default.isExecutableFile(atPath: path) { return path }
        throw XCTSkip("A node or bun runtime is needed for the actual JavaScript compatibility fixture.")
    }
    func testUnchangedRealAPIImportsAndNativeEffectProtocol() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let fixture = root.appendingPathComponent("examples/raycast-extension")
        var command = try RaycastSDKCompatibility.inspect(manifestURL: fixture.appendingPathComponent("package.json"), commandName: "greeting", entryURL: fixture.appendingPathComponent("greeting.js"), runtime: installedRuntime())
        XCTAssertFalse(command.enabled)
        XCTAssertThrowsError(try RaycastSDKCompatibility.execute(command, runner: AutomationProcess()))
        command.enabled = true; command.values["name"] = "Kebloop"
        let response = try RaycastSDKCompatibility.execute(command, runner: AutomationProcess())
        XCTAssertEqual(response.effects.map(\.type), ["copy", "message"])
        XCTAssertEqual(response.effects[0].value, "Hello, Kebloop!")
        XCTAssertEqual(response.effects[1].value, "Greeting copied")
        XCTAssertTrue(try String(contentsOf: fixture.appendingPathComponent("greeting.js"), encoding: .utf8).contains("from \"@raycast/api\""))
    }
    func testUUIDCommandUsesUnchangedSDKAndBuiltinNodeModule() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let fixture = root.appendingPathComponent("examples/raycast-extension")
        var command = try RaycastSDKCompatibility.inspect(manifestURL: fixture.appendingPathComponent("package.json"),
            commandName: "copy-uuid", entryURL: fixture.appendingPathComponent("copy-uuid.js"), runtime: installedRuntime())
        command.enabled = true
        let response = try RaycastSDKCompatibility.execute(command, runner: AutomationProcess())
        XCTAssertEqual(response.effects.map(\.type), ["copy", "message"])
        XCTAssertNotNil(UUID(uuidString: response.effects[0].value))
        XCTAssertEqual(response.effects[1].value, "UUID copied")
    }

    func testUnsupportedSDKAPIIsExplicitFailureWithoutEffects() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("raycast-sdk-test-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = directory.appendingPathComponent("command.js")
        try "import { AI } from '@raycast/api'; export default async function Command() { await AI.ask('hi'); }".write(to: source, atomically: true, encoding: .utf8)
        let command = RaycastSDKCommand(title: "Unsupported", source: source.path, runtime: try installedRuntime(), preferences: [], enabled: true)
        XCTAssertThrowsError(try RaycastSDKCompatibility.execute(command, runner: AutomationProcess())) { error in
            XCTAssertTrue(error.localizedDescription.contains("Unsupported API"))
        }
    }
    func testHTTPOnlyOpenAndRequiredPreferences() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("raycast-sdk-open-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = directory.appendingPathComponent("command.js")
        try "import { open } from '@raycast/api'; export default async function Command() { await open('file:///tmp/private'); }".write(to: source, atomically: true, encoding: .utf8)
        let runtime = try installedRuntime()
        let command = RaycastSDKCommand(title: "Unsafe", source: source.path, runtime: runtime, preferences: [], enabled: true)
        XCTAssertThrowsError(try RaycastSDKCompatibility.execute(command, runner: AutomationProcess()))
        let preference = RaycastSDKPreference(name: "token", title: "Token", type: "textfield", required: true, default: nil)
        let required = RaycastSDKCommand(title: "Required", source: source.path, runtime: runtime, preferences: [preference], enabled: true)
        XCTAssertThrowsError(try RaycastSDKCompatibility.execute(required, runner: AutomationProcess())) { XCTAssertTrue($0.localizedDescription.contains("required")) }
    }
}
