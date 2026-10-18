import AppKit
import XCTest
@testable import Kebloop

@MainActor
final class AIHandoffTests: XCTestCase {
    func testProvidersExposeWebsiteTargetsAndHandoffGuidance() throws {
        XCTAssertEqual(AIProvider.allCases, [.openai, .claude, .local])
        XCTAssertEqual(try XCTUnwrap(AIProvider.openai.websiteURL).absoluteString, "https://chatgpt.com/")
        XCTAssertEqual(try XCTUnwrap(AIProvider.claude.websiteURL).absoluteString, "https://claude.ai/new")
        XCTAssertNil(AIProvider.local.websiteURL)
        XCTAssertEqual(AIProvider.openai.settingsTitle, "OpenAI (ChatGPT)")
        XCTAssertEqual(AIProvider.local.rawValue, "local")
        XCTAssertEqual(AIProvider.local.title, L("Local Assistant"))
        XCTAssertEqual(AIProvider.local.settingsTitle, L("Local Assistant"))
        XCTAssertEqual(AIProvider.openai.explanation, L("Opens ChatGPT with your query already filled in."))
        XCTAssertEqual(AIProvider.claude.explanation, L("Opens Claude with your query in the link. You may need to sign in first."))
        XCTAssertEqual(AIProvider.local.explanation, L("Local Assistant chats inline in Kebloop using Apple Foundation Models on macOS 26 with Apple Intelligence. Web search and other online tools send requests over the internet."))
    }

    func testProviderPreferencePersistsAndInvalidValueFallsBackToOpenAI() throws {
        let (defaults, suite) = try makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set("unknown-provider", forKey: AIHandoffService.providerStorageKey)

        let service = AIHandoffService(defaults: defaults, openURL: { _ in true })
        XCTAssertEqual(service.provider, .openai)
        XCTAssertEqual(defaults.string(forKey: AIHandoffService.providerStorageKey), AIProvider.openai.rawValue)

        service.provider = .claude
        XCTAssertEqual(AIHandoffService(defaults: defaults).provider, .claude)
        service.provider = .local
        XCTAssertEqual(defaults.string(forKey: AIHandoffService.providerStorageKey), "local")
        XCTAssertEqual(AIHandoffService(defaults: defaults).provider, .local)
    }

    func testCloudHandoffCopiesTrimmedPromptMarksItTransientAndOpensSite() throws {
        let (defaults, suite) = try makeDefaults()
        let pasteboard = NSPasteboard(name: NSPasteboard.Name(suite))
        defer {
            pasteboard.releaseGlobally()
            defaults.removePersistentDomain(forName: suite)
        }
        var openedURLs: [URL] = []
        let service = AIHandoffService(
            defaults: defaults,
            pasteboard: pasteboard,
            openURL: { openedURLs.append($0); return true }
        )
        service.provider = .claude
        var result: String? = "not completed"

        service.handoff("\n  Héllo 🌍\nsecond line  \t\n") { result = $0 }

        XCTAssertNil(result)
        XCTAssertEqual(openedURLs, [AIProvider.claude.promptURL("Héllo 🌍\nsecond line")])
        XCTAssertEqual(pasteboard.string(forType: .string), "Héllo 🌍\nsecond line")
        XCTAssertTrue(pasteboard.types?.contains(AIHandoffService.transientPasteboardType) == true)
    }

    func testEmptyPromptIsRejectedWithoutOpeningProvider() throws {
        let (defaults, suite) = try makeDefaults()
        let pasteboard = NSPasteboard(name: NSPasteboard.Name(suite))
        defer {
            pasteboard.releaseGlobally()
            defaults.removePersistentDomain(forName: suite)
        }
        var openCalled = false
        let service = AIHandoffService(defaults: defaults, pasteboard: pasteboard, openURL: { _ in
            openCalled = true
            return true
        })
        var result: String?
        service.handoff(" \n\t ") { result = $0 }

        XCTAssertEqual(result, "Enter a prompt first.")
        XCTAssertFalse(openCalled)
        XCTAssertNil(pasteboard.string(forType: .string))
    }

    func testLocalProviderDirectsToInlineChatWithoutHandoffOrClipboardMutation() throws {
        let (defaults, suite) = try makeDefaults()
        let pasteboard = NSPasteboard(name: NSPasteboard.Name(suite))
        defer {
            pasteboard.releaseGlobally()
            defaults.removePersistentDomain(forName: suite)
        }
        pasteboard.clearContents()
        XCTAssertTrue(pasteboard.setString("leave this text alone", forType: .string))
        var openedURLs: [URL] = []
        let service = AIHandoffService(
            defaults: defaults,
            pasteboard: pasteboard,
            openURL: { openedURLs.append($0); return true }
        )
        service.provider = .local
        var result: String?

        service.handoff("summarize this") { result = $0 }

        XCTAssertEqual(result, L("Local Assistant chat is available inline in the launcher. Press Tab on a query to open it."))
        XCTAssertTrue(openedURLs.isEmpty)
        XCTAssertEqual(pasteboard.string(forType: .string), "leave this text alone")
        XCTAssertFalse(pasteboard.types?.contains(AIHandoffService.transientPasteboardType) == true)
    }

    private func makeDefaults() throws -> (UserDefaults, String) {
        let suite = "KebloopAIHandoffTests.\(UUID().uuidString)"
        return (try XCTUnwrap(UserDefaults(suiteName: suite)), suite)
    }
}
