import XCTest
import Carbon
@testable import Kebloop

final class CommandRegistryTests: XCTestCase {
    @MainActor
    func testRecorderPreservesReservationsAcrossAppAndCommandBindings() throws {
        let hotkey = HotkeyService()
        let modifiers = UInt32(cmdKey | controlKey | optionKey | shiftKey)
        XCTAssertTrue(hotkey.registerBinding(id: "app", keyCode: UInt32(kVK_F19), modifiers: modifiers) {})
        hotkey.beginRecording()
        XCTAssertTrue(hotkey.conflictsWithBinding(id: "command:emoji", keyCode: UInt32(kVK_F19), modifiers: modifiers))
        XCTAssertFalse(hotkey.registerBinding(id: "command:emoji", keyCode: UInt32(kVK_F19), modifiers: modifiers) {})
        hotkey.endRecording()
        hotkey.unregisterBinding(id: "app")
        XCTAssertFalse(hotkey.conflictsWithBinding(id: "command:emoji", keyCode: UInt32(kVK_F19), modifiers: modifiers))
    }
    @MainActor
    func testCommandPersistenceRemovalAndDuplicateConflict() throws {
        let suite = "KebloopCommands.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let hotkey = HotkeyService()
        let store = CommandShortcutStore(hotkey: hotkey, defaults: defaults)
        let modifiers = UInt32(cmdKey | controlKey | shiftKey)
        store.suspend()
        XCTAssertTrue(store.assign(commandID: "emoji", keyCode: UInt32(kVK_F19), modifiers: modifiers, label: "Test"))
        XCTAssertFalse(store.assign(commandID: "help", keyCode: UInt32(kVK_F19), modifiers: modifiers, label: "Test"))
        store.resume()
        store.reload()
        XCTAssertEqual(store.bindings.first?.id, "emoji")
        store.remove(id: "emoji")
        store.reload()
        XCTAssertTrue(store.bindings.isEmpty)
        XCTAssertFalse(hotkey.conflictsWithBinding(id: "", keyCode: UInt32(kVK_F19), modifiers: modifiers))
    }
    func testStableRegistryIdentifiersAreUnique() {
        XCTAssertEqual(Set(LauncherCommand.allCases.map(\.id)).count, LauncherCommand.allCases.count)
        XCTAssertTrue(LauncherCommand.allCases.allSatisfy { !$0.title.isEmpty && !$0.symbol.isEmpty && !$0.aliases.isEmpty })
    }

    func testChatHistoryCommandMetadata() {
        XCTAssertEqual(LauncherCommand.chatHistory.id, "chatHistory")
        XCTAssertEqual(LauncherCommand.chatHistory.title, L("Chat History"))
        XCTAssertEqual(LauncherCommand.chatHistory.symbol, "bubble.left.and.bubble.right")
        XCTAssertEqual(LauncherCommand.chatHistory.aliases, ["chat history", "conversations", "assistant history", "historia czatów", "rozmowy"])
    }
}

extension CommandRegistryTests {
    @MainActor
    func testDisabledCommandReleasesActiveBindingAndRetainsPreference() throws {
        let suite = "KebloopDisabledCommands.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let hotkey = HotkeyService()
        let store = CommandShortcutStore(hotkey: hotkey, defaults: defaults)
        let key = UInt32(kVK_F19), modifiers = UInt32(cmdKey | controlKey | shiftKey)
        XCTAssertTrue(store.assign(commandID: "emoji", keyCode: key, modifiers: modifiers, label: "Test"))
        XCTAssertTrue(hotkey.conflictsWithBinding(id: "other", keyCode: key, modifiers: modifiers))
        store.isEnabled = { _ in false }; store.resume()
        XCTAssertFalse(hotkey.conflictsWithBinding(id: "other", keyCode: key, modifiers: modifiers))
        XCTAssertEqual(store.bindings.first?.id, "emoji")
        store.isEnabled = { _ in true }; store.resume()
        XCTAssertTrue(hotkey.conflictsWithBinding(id: "other", keyCode: key, modifiers: modifiers))
        store.remove(id: "emoji")
    }
}

extension CommandRegistryTests {
    @MainActor
    func testUnavailableReassignmentRetainsPreviousPreferenceAndReservation() throws {
        let suite = "KebloopCommandRollback.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let hotkey = HotkeyService()
        let store = CommandShortcutStore(hotkey: hotkey, defaults: defaults)
        let modifiers = UInt32(cmdKey | controlKey | optionKey | shiftKey)
        XCTAssertTrue(store.assign(commandID: "emoji", keyCode: UInt32(kVK_F19), modifiers: modifiers, label: "Original"))
        var external: EventHotKeyRef?
        let identifier = EventHotKeyID(signature: 0x54455354, id: 1)
        XCTAssertEqual(RegisterEventHotKey(UInt32(kVK_F20), modifiers, identifier, GetApplicationEventTarget(), 0, &external), noErr)
        defer { if let external { UnregisterEventHotKey(external) } }
        store.suspend()
        XCTAssertFalse(store.assign(commandID: "emoji", keyCode: UInt32(kVK_F20), modifiers: modifiers, label: "Unavailable"))
        XCTAssertEqual(store.bindings.first?.label, "Original")
        XCTAssertNotNil(store.recorderConflict)
        store.resume()
        XCTAssertTrue(hotkey.conflictsWithBinding(id: "other", keyCode: UInt32(kVK_F19), modifiers: modifiers))
        store.remove(id: "emoji")
    }
}

extension CommandRegistryTests {
    @MainActor
    func testWindowPresetCoversEveryActionWithUniqueShortcuts() {
        let bindings = WindowShortcutPreset.bindings
        let windowCommands = LauncherCommand.allCases.filter { $0.windowAction != nil }
        XCTAssertEqual(Set(bindings.map(\.id)), Set(windowCommands.map(\.id)))
        XCTAssertEqual(bindings.count, WindowManagementService.Action.allCases.count)
        XCTAssertEqual(Set(bindings.map { "\($0.keyCode):\($0.modifiers)" }).count, bindings.count)
        XCTAssertTrue(bindings.allSatisfy { $0.modifiers & UInt32(controlKey | optionKey) == UInt32(controlKey | optionKey) })
    }

    @MainActor
    func testWindowShortcutsPersistAndRejectConflictingAppBinding() throws {
        let suite = "WindowShortcuts.\(UUID())"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let hotkey = HotkeyService()
        let store = CommandShortcutStore(hotkey: hotkey, defaults: defaults)
        defer { store.unregisterAll() }
        let key = UInt32(kVK_F19), modifiers = UInt32(controlKey | optionKey | cmdKey | shiftKey)
        XCTAssertTrue(hotkey.registerBinding(id: "test-app", keyCode: key, modifiers: modifiers) {})
        defer { hotkey.unregisterBinding(id: "test-app") }
        store.suspend()
        XCTAssertFalse(store.assign(commandID: LauncherCommand.topLeftQuarter.id, keyCode: key, modifiers: modifiers, label: "Conflict"))
        XCTAssertTrue(store.bindings.isEmpty)
        hotkey.unregisterBinding(id: "test-app")
        XCTAssertTrue(store.assign(commandID: LauncherCommand.topLeftQuarter.id, keyCode: key, modifiers: modifiers, label: "Test shortcut"))
        store.resume()
        store.reload()
        XCTAssertEqual(store.bindings.first?.id, LauncherCommand.topLeftQuarter.id)
        XCTAssertEqual(store.bindings.first?.label, "Test shortcut")
    }
}
