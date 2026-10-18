import XCTest
@testable import Kebloop

final class AppleShortcutsTests: XCTestCase {
    @MainActor
    func testListParsesStableIdentifiersAndNamesWithParentheses() {
        let list = AppleShortcutsService.parseList("A shortcut (work) (C46E75DB-FC26-4B25-A70A-6A3304D0D585)\nBad line\nNot an ID (invalid)")
        XCTAssertEqual(list.count, 1)
        XCTAssertEqual(list.first?.name, "A shortcut (work)")
        XCTAssertEqual(list.first?.id, "C46E75DB-FC26-4B25-A70A-6A3304D0D585")
    }
    @MainActor
    func testSelectionPersistenceAndUnknownExecutionFailure() throws {
        let suite = "KebloopAppleShortcuts.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let service = AppleShortcutsService(defaults: defaults)
        let shortcut = AppleShortcut(id: UUID().uuidString, name: "Example")
        service.select(shortcut); service.select(shortcut)
        XCTAssertEqual(service.selected.count, 1)
        XCTAssertEqual(AppleShortcutsService(defaults: defaults).selected.first?.id, shortcut.id)
        var failure: String?
        service.run(id: "unknown") { failure = $0 }
        XCTAssertNotNil(failure)
        XCTAssertFalse(service.isRunning)
        service.cancel()
        service.remove(id: shortcut.id)
        XCTAssertTrue(AppleShortcutsService(defaults: defaults).selected.isEmpty)
    }
}

extension AppleShortcutsTests {
    @MainActor
    func testTextInputFailuresAndCancellationCompleteWithoutBlocking() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let executable = directory.appendingPathComponent("shortcuts")
        let script = """
        #!/bin/sh
        if [ "$2" = "slow" ]; then exec /bin/sleep 30; fi
        if [ "$2" = "failure" ]; then echo 'permission denied'; exit 1; fi
        if [ "$3" != "--input-path" ]; then exit 2; fi
        /usr/bin/cmp "$4" "\(directory.appendingPathComponent("expected.txt").path)"
        printf 'result from shortcut\\n'
        """
        try script.write(to: executable, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let input = "Łódź & cats\nsecond line"
        try input.write(to: directory.appendingPathComponent("expected.txt"), atomically: true, encoding: .utf8)
        let suite = "KebloopAppleExecution.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let service = AppleShortcutsService(defaults: defaults, executableURL: executable)
        for id in ["input", "failure", "slow"] { service.select(AppleShortcut(id: id, name: id)) }
        let ran = expectation(description: "Passed UTF8 input and captured output")
        service.run(id: "input", input: input) { error in
            XCTAssertNil(error)
            XCTAssertEqual(service.output, "result from shortcut\n")
            ran.fulfill()
        }
        XCTAssertTrue(service.isRunning)
        await fulfillment(of: [ran], timeout: 3)
        let failed = expectation(description: "Execution failure")
        service.run(id: "failure") { error in
            XCTAssertTrue(error?.contains("permission denied") == true)
            XCTAssertNil(service.output)
            failed.fulfill()
        }
        await fulfillment(of: [failed], timeout: 3)
        let cancelled = expectation(description: "Cancellation callback")
        service.run(id: "slow") { error in XCTAssertEqual(error, "Shortcut cancelled."); cancelled.fulfill() }
        service.cancel()
        XCTAssertFalse(service.isRunning)
        await fulfillment(of: [cancelled], timeout: 1)
    }
}

extension AppleShortcutsTests {
    @MainActor
    func testRefreshRetainsStableSelectionAcrossRenameAndReportsDeletion() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let executable = directory.appendingPathComponent("shortcuts")
        let list = directory.appendingPathComponent("list.txt")
        try "#!/bin/sh\n/bin/cat '\(list.path)'\n".write(to: executable, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let suite = "KebloopAppleRefresh.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let id = UUID().uuidString
        let service = AppleShortcutsService(defaults: defaults, executableURL: executable)
        service.select(AppleShortcut(id: id, name: "Original"))
        try "Renamed (\(id))\n".write(to: list, atomically: true, encoding: .utf8)
        service.refresh()
        for _ in 0..<200 where service.isLoading { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertFalse(service.isLoading)
        XCTAssertEqual(service.selected.first?.name, "Renamed")
        XCTAssertNil(service.errors[id])
        try "".write(to: list, atomically: true, encoding: .utf8)
        service.refresh()
        for _ in 0..<200 where service.isLoading { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertFalse(service.isLoading)
        XCTAssertEqual(service.selected.first?.id, id)
        XCTAssertTrue(service.errors[id]?.contains("missing") == true)
        var error: String?
        service.run(id: id) { error = $0 }
        XCTAssertNotNil(error)
        XCTAssertFalse(service.isRunning)
    }
}
