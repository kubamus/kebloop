import AppKit
import XCTest
@testable import Kebloop

@MainActor
final class LocalAssistantIntegrationTests: XCTestCase {
    func testInstalledVSCodeAliasesAndMetadataUseTheActualAppRegistry() async throws {
        try await withLauncher { launcher, _, _, backend in
            struct AppContext: Decodable { let id: String; let name: String; let path: String; let aliases: [String] }
            let json = try await launcher.executeAssistantAction(id: "application.search", input: "vscode")
            let contexts = try JSONDecoder().decode([AppContext].self, from: Data(json.utf8))
            guard let code = contexts.first(where: { $0.aliases.contains { $0.lowercased() == "vscode" } }) else {
                throw XCTSkip("VS Code must be installed for the registry integration check.")
            }
            let catalog = launcher.assistantCatalog(query: "vscode")
            XCTAssertTrue(catalog.contains(code.id))
            XCTAssertTrue(catalog.contains("aliases"))
            for name in ["vscode", "vs code", "Visual Studio Code"] {
                _ = try await launcher.executeAssistantAction(id: "application.open", input: name)
                XCTAssertEqual(backend.launchedAppIDs.last, String(code.id.dropFirst(4)))
            }
        }
    }

    func testLiveModelOpensVSCodeThroughTheRealAppRegistryWhenRequested() async throws {
        guard ProcessInfo.processInfo.environment["KEBLOOP_TEST_LOCAL_MODEL"] == "1" else {
            throw XCTSkip("Set KEBLOOP_TEST_LOCAL_MODEL=1 to exercise the installed Apple model.")
        }
        try await withLauncher { launcher, _, _, backend in
            let context = try await launcher.executeAssistantAction(id: "application.search", input: "vscode")
            guard context.lowercased().contains("vscode") else { throw XCTSkip("VS Code must be installed.") }
            let tools = LocalAssistantTools()
            tools.catalogHandler = { query in launcher.assistantCatalog(query: query) }
            tools.executeHandler = { id, input in try await launcher.executeAssistantAction(id: id, input: input) }
            let service = LocalAssistantService(tools: tools, history: LocalAssistantHistoryStore(storageURL: nil))
            if let unavailable = service.availabilityMessage { throw XCTSkip(unavailable) }
            defer { service.cancel() }
            service.send("open vscode")
            for _ in 0..<240 {
                if !service.isResponding { break }
                try await Task.sleep(for: .milliseconds(250))
            }
            XCTAssertFalse(service.isResponding)
            XCTAssertNil(service.status)
            XCTAssertEqual(backend.launchedAppIDs.count, 1, service.messages.last?.text ?? "No response")
        }
    }

    func testLiveModelOpensApplicationsByNameWhenRequested() async throws {
        guard ProcessInfo.processInfo.environment["KEBLOOP_TEST_LOCAL_MODEL"] == "1" else {
            throw XCTSkip("Set KEBLOOP_TEST_LOCAL_MODEL=1 to exercise the installed Apple model.")
        }
        let tools = LocalAssistantTools()
        var succeeded = false
        tools.catalogHandler = { _ in #"[{"id":"app:/Applications/Spotify.app","title":"Spotify"}]"# }
        tools.executeHandler = { id, input in
            guard id == "application.open", input.localizedCaseInsensitiveCompare("Spotify") == .orderedSame else {
                throw AutomationFailure.invalid("Use open_application with name Spotify.")
            }
            succeeded = true
            return "Opened Spotify."
        }
        let service = LocalAssistantService(tools: tools, history: LocalAssistantHistoryStore(storageURL: nil))
        if let unavailable = service.availabilityMessage { throw XCTSkip(unavailable) }
        defer { service.cancel() }
        service.send("Open Spotify")
        for _ in 0..<240 {
            if !service.isResponding { break }
            try await Task.sleep(for: .milliseconds(250))
        }
        XCTAssertFalse(service.isResponding)
        XCTAssertNil(service.status)
        XCTAssertTrue(succeeded, service.messages.last?.text ?? "No response")
    }

    func testLiveAppleModelSmokeTestWhenRequested() async throws {
        guard ProcessInfo.processInfo.environment["KEBLOOP_TEST_LOCAL_MODEL"] == "1" else {
            throw XCTSkip("Set KEBLOOP_TEST_LOCAL_MODEL=1 to exercise the installed Apple model.")
        }
        let tools = LocalAssistantTools()
        var executedIDs: [String] = []
        tools.catalogHandler = { _ in #"[{"id":"calculate","title":"Calculate arithmetic. Input is an expression."}]"# }
        tools.executeHandler = { id, _ in
            guard id == "calculate" else { throw AutomationFailure.invalid("Unknown ID. Use the id field from the catalog.") }
            executedIDs.append(id)
            return "4553"
        }
        let service = LocalAssistantService(tools: tools, history: LocalAssistantHistoryStore(storageURL: nil))
        if let unavailable = service.availabilityMessage { throw XCTSkip(unavailable) }
        defer { service.cancel() }
        service.send("Use the Kebloop action tools to calculate 157 * 29. Discover the exact action ID from the catalog first.")
        for _ in 0..<240 {
            if !service.isResponding { break }
            try await Task.sleep(for: .milliseconds(250))
        }
        XCTAssertFalse(service.isResponding, "Local model timed out")
        XCTAssertNil(service.status)
        XCTAssertFalse(service.messages.last?.text.isEmpty ?? true)
        XCTAssertEqual(executedIDs, ["calculate"])
        XCTAssertTrue(service.messages.last?.text.contains("4553") == true)
        let savedConversationID = service.currentConversationID
        service.reset()
        XCTAssertTrue(service.messages.isEmpty)
        XCTAssertTrue(service.resumeConversation(id: savedConversationID))
        service.send("What was the number you just calculated?")
        for _ in 0..<240 {
            if !service.isResponding { break }
            try await Task.sleep(for: .milliseconds(250))
        }
        XCTAssertFalse(service.isResponding)
        XCTAssertNil(service.status)
        XCTAssertTrue(service.messages.last?.text.contains("4553") == true)
        XCTAssertEqual(executedIDs, ["calculate"], "Resuming must not rerun earlier tool calls")
    }

    func testChatStaysInLauncherAndSendsFollowUps() async throws {
        try await withLauncher { launcher, _, _, backend in
            launcher.showLocalAssistant(query: "First question")
            await settle(launcher.localAssistant)
            XCTAssertEqual(launcher.currentModeID, "localAssistant")
            XCTAssertTrue(launcher.isVisible)
            XCTAssertEqual(launcher.currentQuery, "")
            XCTAssertEqual(launcher.localAssistant.messages.last?.text, "Answer: First question")

            launcher.setQuery("Follow-up")
            launcher.performSelected()
            await settle(launcher.localAssistant)
            XCTAssertEqual(backend.prompts, ["First question", "Follow-up"])
            XCTAssertEqual(launcher.localAssistant.messages.count, 4)

            let escape = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
                timestamp: 0, windowNumber: 0, context: nil, characters: "\u{1b}",
                charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
            XCTAssertTrue(launcher.handleUtilityKey(escape))
            XCTAssertEqual(launcher.currentModeID, "root")
            let conversationID = launcher.localAssistant.currentConversationID
            launcher.showLocalAssistant()
            XCTAssertTrue(launcher.localAssistant.messages.isEmpty)
            XCTAssertNotEqual(launcher.localAssistant.currentConversationID, conversationID)
            XCTAssertEqual(launcher.localAssistant.history.conversations.first?.messages.count, 4)

            launcher.executeCommand(id: LauncherCommand.chatHistory.id)
            XCTAssertEqual(launcher.currentModeID, "chatHistory")
            launcher.setQuery("First question")
            launcher.performSelected()
            XCTAssertEqual(launcher.currentModeID, "localAssistant")
            XCTAssertEqual(launcher.localAssistant.currentConversationID, conversationID)
            XCTAssertEqual(launcher.localAssistant.messages.count, 4)
            XCTAssertEqual(backend.restoredMessages.count, 4)
            launcher.setQuery("Continue old chat")
            launcher.performSelected()
            await settle(launcher.localAssistant)
            XCTAssertEqual(launcher.localAssistant.messages.count, 6)
            XCTAssertEqual(launcher.localAssistant.history.conversations.count, 1)

            launcher.showChatHistory()
            launcher.removeLocalAssistantConversation(id: conversationID)
            XCTAssertTrue(launcher.localAssistant.history.conversations.isEmpty)
            XCTAssertTrue(launcher.localAssistant.messages.isEmpty)
            launcher.showLocalAssistant(query: "Another conversation")
            await settle(launcher.localAssistant)
            XCTAssertEqual(launcher.localAssistant.history.conversations.count, 1)
            XCTAssertNotEqual(launcher.localAssistant.history.conversations.first?.id, conversationID)
        }
    }

    func testCatalogAndExecutionRespectEnabledCommands() async throws {
        try await withLauncher { launcher, personalization, _, _ in
            XCTAssertTrue(launcher.assistantCatalog(query: "").contains("toggleMute"))
            XCTAssertFalse(launcher.assistantCatalog(query: "toggleMute").contains("notes.create"))
            let calculation = try await launcher.executeAssistantAction(id: "calculate", input: "2 + 2")
            XCTAssertEqual(calculation, "4")
            var settings = personalization.settings
            settings.commandsEnabled = false
            personalization.settings = settings
            XCTAssertFalse(launcher.assistantCatalog(query: "").contains("toggleMute"))
            do {
                _ = try await launcher.executeAssistantAction(id: "calculate", input: "2 + 2")
                XCTFail("Disabled commands must not execute")
            } catch { XCTAssertTrue(error.localizedDescription.contains("disabled")) }
        }
    }

    func testTabStartsFreshChatsAndHistoryCanResumeThem() async throws {
        try await withLauncher { launcher, _, _, _ in
            guard !NSApp.isFullKeyboardAccessEnabled else { throw XCTSkip("Tab navigates controls with Full Keyboard Access.") }
            let tab = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
                timestamp: 0, windowNumber: 0, context: nil, characters: "\t",
                charactersIgnoringModifiers: "\t", isARepeat: false, keyCode: 48))
            launcher.showLocalAssistant(query: "Original chat")
            await settle(launcher.localAssistant)
            let firstID = launcher.localAssistant.currentConversationID
            launcher.setQuery("A different chat")
            XCTAssertTrue(launcher.handleUtilityKey(tab))
            await settle(launcher.localAssistant)
            XCTAssertNotEqual(launcher.localAssistant.currentConversationID, firstID)
            XCTAssertEqual(launcher.localAssistant.messages.first?.text, "A different chat")
            XCTAssertEqual(launcher.localAssistant.messages.count, 2)
            XCTAssertEqual(launcher.localAssistant.history.conversations.count, 2)
            launcher.showChatHistory()
            launcher.setQuery("Original")
            launcher.performSelected()
            XCTAssertEqual(launcher.localAssistant.currentConversationID, firstID)
            launcher.showChatHistory()
            XCTAssertTrue(launcher.handleUtilityKey(tab))
            XCTAssertEqual(launcher.currentModeID, "localAssistant")
            XCTAssertTrue(launcher.localAssistant.messages.isEmpty)
            XCTAssertEqual(launcher.localAssistant.history.conversations.count, 2)
        }
    }

    func testScriptArgumentsRemainSeparateAndOutputReturnsToChat() async throws {
        try await withLauncher { launcher, _, _, _ in
            let output = try await launcher.executeAssistantAction(id: "script:assistant-echo", input: #"["hello world", "café"]"#)
            XCTAssertEqual(output, "configured hello world café\n")
        }
    }

    func testOpeningApplicationsUsesRegistryAndRespectsDisabledApps() async throws {
        try await withLauncher { launcher, personalization, _, backend in
            struct Action: Decodable { let id: String; let title: String }
            let matches = try JSONDecoder().decode([Action].self, from: Data(launcher.assistantCatalog(query: "open Safari").utf8))
                .filter { $0.id.hasPrefix("app:") }
            guard matches.count == 1, let app = matches.first else { throw XCTSkip("A unique indexed Safari app is required.") }
            let result = try await launcher.executeAssistantAction(id: "application.open", input: app.title)
            XCTAssertEqual(result, "Opened \(app.title).")
            XCTAssertEqual(backend.launchedAppIDs, [String(app.id.dropFirst(4))])
            personalization.settings.appsEnabled = false
            do {
                _ = try await launcher.executeAssistantAction(id: "application.open", input: app.title)
                XCTFail("Disabled apps must not open")
            } catch { XCTAssertTrue(error.localizedDescription.contains("disabled")) }
            XCTAssertEqual(backend.launchedAppIDs.count, 1)
        }
    }

    func testNoteToolsReturnActualContentsAndRejectUnknownActions() async throws {
        try await withLauncher { launcher, _, notes, _ in
            let created = try await launcher.executeAssistantAction(id: "notes.create", input: "# Shopping\nApples")
            XCTAssertTrue(created.contains("Shopping"))
            XCTAssertEqual(notes.notes.count, 1)
            let found = try await launcher.executeAssistantAction(id: "notes.search", input: "Apples")
            XCTAssertTrue(found.contains("# Shopping\nApples"))
            do {
                _ = try await launcher.executeAssistantAction(id: "invented-action", input: "")
                XCTFail("Unknown actions must fail")
            } catch { XCTAssertTrue(error.localizedDescription.contains("Unknown or disabled")) }
        }
    }

    private func settle(_ service: LocalAssistantService) async {
        for _ in 0..<100 {
            if !service.isResponding { return }
            await Task.yield()
        }
    }

    private func withLauncher(_ body: (LauncherPanelController, PersonalizationStore, MarkdownNoteStore, IntegrationAssistantBackend) async throws -> Void) async throws {
        _ = NSApplication.shared
        let suite = "LocalAssistantIntegration.\(UUID())"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defaults.set(false, forKey: "clipboardHistoryEnabled")
        let pasteboard = NSPasteboard(name: .init(suite))
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(suite)
        defer {
            defaults.removePersistentDomain(forName: suite)
            pasteboard.releaseGlobally()
            try? FileManager.default.removeItem(at: directory)
        }
        let notes = MarkdownNoteStore(directory: directory.appendingPathComponent("Notes"))
        let personalization = PersonalizationStore(defaults: defaults)
        let scripts = ScriptCommandStore(defaults: defaults)
        try scripts.save(ScriptCommand(id: "assistant-echo", title: "Echo", description: "Echo arguments", executable: "/bin/echo", arguments: ["configured"], enabled: true))
        let backend = IntegrationAssistantBackend()
        let assistant = LocalAssistantService(backend: backend)
        let launcher = LauncherPanelController(index: ApplicationIndex(), quicklinks: QuicklinkStore(defaults: defaults),
            clipboard: ClipboardHistoryStore(defaults: defaults, pasteboard: pasteboard, storageURL: directory.appendingPathComponent("history.json")),
            aliases: AppAliasStore(defaults: defaults), ai: AIHandoffService(defaults: defaults, pasteboard: pasteboard),
            personalization: personalization, appleShortcuts: AppleShortcutsService(defaults: defaults), scripts: scripts,
            launchApplication: { app, completion in backend.launchedAppIDs.append(app.id); completion(nil) },
            localAssistant: assistant)
        launcher.noteStore = { notes }
        defer { launcher.dismiss(restoreFocus: false); notes.flushAll() }
        try await body(launcher, personalization, notes, backend)
    }
}

@MainActor
private final class IntegrationAssistantBackend: LocalAssistantBackend {
    var availabilityMessage: String?
    var prompts: [String] = []
    var launchedAppIDs: [String] = []
    var restoredMessages: [LocalAssistantMessage] = []
    func streamResponse(to prompt: String, onUpdate: @escaping @MainActor (String) -> Void) async throws -> String {
        prompts.append(prompt)
        onUpdate("Answer: " + prompt)
        return "Answer: " + prompt
    }
    func restore(messages: [LocalAssistantMessage]) { restoredMessages = messages }
    func refreshAvailability() {}
    func cancel() {}
    func reset() { restoredMessages = [] }
}
