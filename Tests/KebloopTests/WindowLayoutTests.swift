import XCTest
@testable import Kebloop

final class WindowLayoutTests: XCTestCase {
    func testNormalizedWindowFrameAdaptsAcrossDisplayBounds() {
        let originalDisplay = CGRect(x: 0, y: 0, width: 1440, height: 900)
        let originalFrame = CGRect(x: 360, y: 225, width: 720, height: 450)
        let saved = SavedWindowFrame(frame: originalFrame, within: originalDisplay)

        XCTAssertEqual(saved.frame(within: originalDisplay), originalFrame)
        XCTAssertEqual(saved.frame(within: CGRect(x: -1600, y: -200, width: 1600, height: 1000)),
                       CGRect(x: -1200, y: 50, width: 800, height: 500))
    }

    func testRestoredFrameIsClampedToCurrentVisibleDisplay() {
        let visible = CGRect(x: -1200, y: 40, width: 900, height: 700)
        let offscreen = CGRect(x: 100, y: -400, width: 1200, height: 1000)

        XCTAssertEqual(SavedWindowFrame.clamp(offscreen, to: visible), visible)
    }

    func testDisplayMatchingUsesIdentifierThenNameThenClosestSize() {
        let saved = SavedLayoutDisplay(identifier: 7, name: "Studio", width: 2560, height: 1440)
        let reordered = [
            SavedLayoutDisplay(identifier: 9, name: "Built-in", width: 1728, height: 1117),
            SavedLayoutDisplay(identifier: 7, name: "Studio Display", width: 2560, height: 1440)
        ]
        XCTAssertEqual(SavedWindowLayoutMatching.displayIndex(for: saved, among: reordered), 1)

        let legacy = SavedLayoutDisplay(identifier: nil, name: "Built-in", width: 1728, height: 1117)
        XCTAssertEqual(SavedWindowLayoutMatching.displayIndex(for: legacy, among: reordered), 0)

        let disconnected = SavedLayoutDisplay(identifier: nil, name: "", width: 3440, height: 1440)
        let choices = [
            SavedLayoutDisplay(identifier: 10, name: "Portrait", width: 1000, height: 1600),
            SavedLayoutDisplay(identifier: 11, name: "Wide", width: 3000, height: 1600)
        ]
        XCTAssertEqual(SavedWindowLayoutMatching.displayIndex(for: disconnected, among: choices), 1)
    }

    func testWindowMatchingHandlesDuplicateTitlesAndPreviouslyMatchedWindows() {
        let titles = ["Document", "Untitled", "Document"]

        XCTAssertEqual(SavedWindowLayoutMatching.matchWindowIndex(title: "Document", ordinal: 1, titles: titles), 2)
        XCTAssertEqual(SavedWindowLayoutMatching.matchWindowIndex(title: "Document", ordinal: 1,
                                                                  titles: titles, usedIndexes: [2]), 0)
        XCTAssertNil(SavedWindowLayoutMatching.matchWindowIndex(title: "Missing", ordinal: 0, titles: titles))
    }

    func testSavedLayoutEncodesAndDecodesDisplayAndWindowIdentity() throws {
        let placement = SavedWindowPlacement(bundleIdentifier: "com.example.editor", applicationName: "Editor",
                                             title: "Draft.md", ordinal: 0,
                                             display: SavedLayoutDisplay(identifier: 42, name: "Desk", width: 1200, height: 800),
                                             frame: SavedWindowFrame(frame: CGRect(x: 120, y: 80, width: 600, height: 400),
                                                                     within: CGRect(x: 0, y: 0, width: 1200, height: 800)))
        let layout = SavedWindowLayout(name: "Writing", windows: [placement])

        let roundTrip = try JSONDecoder().decode(SavedWindowLayout.self, from: JSONEncoder().encode(layout))

        XCTAssertEqual(roundTrip, layout)
        XCTAssertEqual(roundTrip.windows[0].id, "com.example.editor:Draft.md:0")
    }
}
