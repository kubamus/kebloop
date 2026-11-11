import XCTest
import KebloopCore
@testable import Kebloop

final class LauncherSearchTests: XCTestCase {
    private let systemID = "/System/Applications/System Settings.app"
    private var entries: [AppSearchEntry] {
        [
            .init(id: "command:settings", name: LauncherCommand.settings.title, path: "", aliases: LauncherCommand.settings.aliases),
            .init(id: systemID, name: "System Settings", path: systemID, aliases: LauncherSearch.settingsAliases),
            .init(id: "other", name: "Settings Editor", path: "")
        ]
    }

    func testSystemSettingsWinsForGenericQueriesDespiteCommandUsage() throws {
        for query in ["settings", " Settings ", "set", "preferences", "pref", "ustawienia", "preferencje", "system settings"] {
            let ids = LauncherSearch.ranked(query: query, entries: entries,
                usage: ["command:settings": 50_000], systemSettingsIDs: [systemID]).map(\.id)
            let systemIndex = try XCTUnwrap(ids.firstIndex(of: systemID), query)
            if let kebloopIndex = ids.firstIndex(of: "command:settings") {
                XCTAssertLessThan(systemIndex, kebloopIndex, query)
            }
        }
    }

    func testExplicitKebloopSettingsAndEmptyQueryKeepNormalRanking() {
        for query in ["kebloop settings", "open settings", ""] {
            let ids = LauncherSearch.ranked(query: query, entries: entries,
                usage: ["command:settings": 50_000], systemSettingsIDs: [systemID]).map(\.id)
            XCTAssertEqual(ids.first, "command:settings", query)
        }
    }

    func testHiddenOrDisabledSystemAppIsNotReintroduced() {
        let results = LauncherSearch.ranked(query: "settings", entries: Array(entries.prefix(1)),
            usage: [:], systemSettingsIDs: [])
        XCTAssertEqual(results.map(\.id), ["command:settings"])
    }
}
