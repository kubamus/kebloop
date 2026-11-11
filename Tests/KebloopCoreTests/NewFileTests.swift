import Foundation
import XCTest
@testable import KebloopCore

final class NewFileTests: XCTestCase {
    private func directory() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("NewFileTests-\(UUID())")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try FileManager.default.removeItem(at: url) }
        return url
    }

    func testCreatesTemplatesAndIncrementsWithoutOverwriting() throws {
        let folder = try directory()
        let type = NewFileType(fileExtension: "json", baseName: "data.json", menuTitle: "JSON", template: "{\n}\n")
        let first = try NewFileCreator.create(type: type, in: folder)
        try Data("keep me".utf8).write(to: first)
        // Directory collisions must be skipped as well as files.
        try FileManager.default.createDirectory(at: folder.appendingPathComponent("data 2.json"), withIntermediateDirectories: false)
        let next = try NewFileCreator.create(type: type, in: folder)
        XCTAssertEqual(first.lastPathComponent, "data.json")
        XCTAssertEqual(next.lastPathComponent, "data 3.json")
        XCTAssertEqual(try String(contentsOf: first, encoding: .utf8), "keep me")
        XCTAssertEqual(try String(contentsOf: next, encoding: .utf8), type.template)
    }

    func testDotfilesAndRichText() throws {
        let folder = try directory()
        let type = NewFileType(fileExtension: "env", baseName: "", menuTitle: "Environment")
        XCTAssertEqual(try NewFileCreator.create(type: type, in: folder).lastPathComponent, ".env")
        XCTAssertEqual(try NewFileCreator.create(type: type, in: folder).lastPathComponent, ".env 2")
        let rtf = try XCTUnwrap(NewFileConfiguration.presets.first { $0.fileExtension == "rtf" })
        let file = try NewFileCreator.create(type: rtf, in: folder)
        XCTAssertEqual(try String(contentsOf: file, encoding: .utf8), "{\\rtf1}")
    }

    func testRejectsPathTraversalAndNormalizesExtension() throws {
        let folder = try directory()
        for ext in ["../txt", "txt/escape", "x:y", "", "💥", "a\0b"] {
            XCTAssertThrowsError(try NewFileCreator.create(type: .init(fileExtension: ext, baseName: "file", menuTitle: ""), in: folder))
        }
        for name in ["../escape", "sub/file", "a:b", "a\0b", "a\nb"] {
            XCTAssertThrowsError(try NewFileCreator.create(type: .init(fileExtension: "txt", baseName: name, menuTitle: ""), in: folder))
        }
        let type = NewFileType(fileExtension: " .TSX ", baseName: "component", menuTitle: "")
        XCTAssertEqual(try NewFileCreator.create(type: type, in: folder).lastPathComponent, "component.tsx")
    }

    func testConfigurationRoundTripsOrderTemplatesAndDisabledTypes() throws {
        let url = try directory().appendingPathComponent("settings/new-file.json")
        XCTAssertEqual(try NewFileConfiguration.load(from: url), NewFileConfiguration())
        var config = NewFileConfiguration(types: [], useSubmenu: true)
        config.types = [
            .init(fileExtension: "tsx", baseName: "Component", menuTitle: "New Component", template: "export default null;", enabled: true),
            NewFileConfiguration.presets[0]
        ]
        config.types[1].enabled = false
        try config.save(to: url)
        XCTAssertEqual(try NewFileConfiguration.load(from: url), config)
        XCTAssertEqual(config.enabledTypes.map(\.fileExtension), ["tsx"])
        config.types = []
        try config.save(to: url)
        XCTAssertTrue(try NewFileConfiguration.load(from: url).enabledTypes.isEmpty)
        try Data("invalid json".utf8).write(to: url)
        XCTAssertThrowsError(try NewFileConfiguration.load(from: url))
    }

    func testConcurrentCreationNeverOverwrites() async throws {
        let folder = try directory()
        let names = try await withThrowingTaskGroup(of: String.self) { group in
            for _ in 0..<20 {
                group.addTask {
                    try NewFileCreator.create(type: NewFileConfiguration.presets[0], in: folder).lastPathComponent
                }
            }
            var names: [String] = []
            for try await name in group { names.append(name) }
            return names
        }
        XCTAssertEqual(Set(names).count, 20)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: folder.path).count, 20)
    }
}
