import Foundation
import XCTest
@testable import Kebloop

final class AssistantApplicationIdentityTests: XCTestCase {
    func testOldApplicationCacheStillDecodesWithoutNewMetadata() throws {
        let data = Data(#"[{"name":"Code","path":"/Applications/Visual Studio Code.app"}]"#.utf8)

        let applications = try JSONDecoder().decode([IndexedApplication].self, from: data)

        XCTAssertEqual(applications, [IndexedApplication(name: "Code", path: "/Applications/Visual Studio Code.app")])
        XCTAssertNil(applications.first?.bundleIdentifier)
        XCTAssertNil(applications.first?.displayName)
    }

    func testScannerCapturesBundleIdentityMetadata() throws {
        try withDirectory { root in
            let appURL = root.appendingPathComponent("Metadata App.app", isDirectory: true)
            try makeApplicationBundle(at: appURL)

            let app = try XCTUnwrap(ApplicationIndex.scanApplications(roots: [root], additionalApplications: []).first)

            XCTAssertEqual(app.name, "Metadata Display Name")
            XCTAssertEqual(app.bundleIdentifier, "com.example.MetadataApp")
            XCTAssertEqual(app.displayName, "Metadata Display Name")
            XCTAssertEqual(app.bundleName, "Metadata Bundle Name")
            XCTAssertEqual(app.executableName, "MetadataExecutable")
        }
    }

    func testVisualStudioCodeIdentityIncludesKnownNames() {
        let app = visualStudioCode()
        let identity = AssistantApplicationIdentity(app: app)

        XCTAssertTrue(identity.aliases.contains("Visual Studio Code"))
        XCTAssertTrue(identity.aliases.contains("VS Code"))
        XCTAssertTrue(identity.aliases.contains("VSCode"))
        XCTAssertTrue(identity.aliases.contains("Code"))
        XCTAssertTrue(identity.matches(query: "VISUAL-STUDIO-CODE"))
    }

    func testResolveMatchesCaseAndPunctuationVariants() {
        let app = visualStudioCode()

        XCTAssertEqual(
            AssistantApplicationIdentity.resolve(query: "vS-CoDe", applications: [app], customAliases: [:]),
            [app]
        )
    }

    func testBundleIdentifierSuffixDoesNotCreateAnAppAlias() {
        let app = IndexedApplication(
            name: "Other Editor",
            path: "/Applications/Other Editor.app",
            bundleIdentifier: "com.example.Code"
        )

        XCTAssertTrue(AssistantApplicationIdentity.resolve(query: "code", applications: [app], customAliases: [:]).isEmpty)
    }

    func testResolveReturnsAmbiguousCanonicalNameMatchesTogether() {
        let stable = visualStudioCode()
        let anotherCodeApp = IndexedApplication(
            name: "Code",
            path: "/Applications/Another Code.app",
            bundleIdentifier: "com.example.AnotherCode"
        )

        let matches = AssistantApplicationIdentity.resolve(
            query: "code",
            applications: [stable, anotherCodeApp],
            customAliases: [:]
        )

        XCTAssertEqual(Set(matches.map(\.path)), Set([stable.path, anotherCodeApp.path]))
    }

    func testVisualStudioCodeInsidersAndVisualStudioRemainDistinct() {
        let stable = visualStudioCode()
        let insiders = IndexedApplication(
            name: "Code - Insiders",
            path: "/Applications/Visual Studio Code - Insiders.app",
            bundleIdentifier: "com.microsoft.VSCodeInsiders"
        )
        let visualStudio = IndexedApplication(
            name: "Visual Studio",
            path: "/Applications/Visual Studio.app",
            bundleIdentifier: "com.microsoft.visualstudio"
        )

        XCTAssertEqual(
            AssistantApplicationIdentity.resolve(query: "vscode", applications: [stable, insiders], customAliases: [:]),
            [stable]
        )
        XCTAssertEqual(
            AssistantApplicationIdentity.resolve(query: "code insiders", applications: [stable, insiders], customAliases: [:]),
            [insiders]
        )
        XCTAssertEqual(
            AssistantApplicationIdentity.resolve(query: "visual studio", applications: [stable, visualStudio], customAliases: [:]),
            [visualStudio]
        )
    }

    func testAcronymCanResolveOneUniqueInstalledApp() {
        let app = visualStudioCode()

        XCTAssertEqual(
            AssistantApplicationIdentity.resolve(query: "vsc", applications: [app], customAliases: [:]),
            [app]
        )
    }

    func testCustomAliasIsUsedForExactResolution() {
        let app = IndexedApplication(name: "Nova", path: "/Applications/Nova.app")

        XCTAssertEqual(
            AssistantApplicationIdentity.resolve(query: "editor", applications: [app], customAliases: [app.path: "Editor"]),
            [app]
        )
    }

    private func visualStudioCode() -> IndexedApplication {
        IndexedApplication(
            name: "Code",
            path: "/Applications/Visual Studio Code.app",
            bundleIdentifier: "com.microsoft.VSCode",
            displayName: "Code",
            bundleName: "Code",
            executableName: "Code"
        )
    }

    private func withDirectory(_ body: (URL) throws -> Void) throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("kebloop-assistant-app-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try body(root)
    }

    private func makeApplicationBundle(at url: URL) throws {
        let contents = url.appendingPathComponent("Contents", isDirectory: true)
        try FileManager.default.createDirectory(at: contents, withIntermediateDirectories: true)
        let info: [String: Any] = [
            "CFBundleExecutable": "MetadataExecutable",
            "CFBundleIdentifier": "com.example.MetadataApp",
            "CFBundleName": "Metadata Bundle Name",
            "CFBundleDisplayName": "Metadata Display Name",
            "CFBundlePackageType": "APPL"
        ]
        let data = try PropertyListSerialization.data(fromPropertyList: info, format: .xml, options: 0)
        try data.write(to: contents.appendingPathComponent("Info.plist"))
    }
}
