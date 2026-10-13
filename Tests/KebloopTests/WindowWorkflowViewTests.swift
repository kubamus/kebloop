import AppKit
import SwiftUI
import XCTest
@testable import Kebloop

final class WindowWorkflowViewTests: XCTestCase {
    @MainActor
    func testMenuSearchHostBuildsWithMissingPreviousApplicationWithoutCheckingOrPromptingForAccess() async {
        var trustChecks = 0
        var permissionPrompts = 0
        let model = WindowMenuSearchModel(application: nil, scope: .menus, accessibilityTrusted: {
            trustChecks += 1
            return false
        })

        await model.refresh()

        XCTAssertEqual(model.status, .failed(L("The previous application is no longer available.")))
        XCTAssertEqual(trustChecks, 0, "Missing-app feedback should be decided before checking Accessibility trust.")
        let host = NSHostingView(rootView: WindowMenuSearchView(
            model: model, scope: .menus, onRequestAccessibilityPermission: { permissionPrompts += 1 }))
        host.frame = NSRect(x: 0, y: 0, width: 560, height: 560)
        host.layoutSubtreeIfNeeded()
        _ = host.fittingSize

        XCTAssertEqual(permissionPrompts, 0)
    }

    @MainActor
    func testWindowSearchHostBuildsWithMissingAccessibilityPermissionWithoutPrompting() async {
        var permissionPrompts = 0
        let model = WindowMenuSearchModel(application: nil, scope: .windows, accessibilityTrusted: { false })

        await model.refresh()

        XCTAssertEqual(model.status, .failed(L("Enable Kebloop in System Settings → Privacy & Security → Accessibility to search windows and menus.")))
        let host = NSHostingView(rootView: WindowMenuSearchView(
            model: model, scope: .windows, onRequestAccessibilityPermission: { permissionPrompts += 1 }))
        host.frame = NSRect(x: 0, y: 0, width: 560, height: 560)
        host.layoutSubtreeIfNeeded()
        _ = host.fittingSize

        XCTAssertEqual(permissionPrompts, 0)
    }

    @MainActor
    func testSavedLayoutsHostUsesAnIsolatedStoreAndPermissionFailureDoesNotTouchWindows() async throws {
        let suite = "KebloopWindowLayoutView.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defaults.removePersistentDomain(forName: suite)
        defer { defaults.removePersistentDomain(forName: suite) }
        var trustChecks = 0
        let store = SavedWindowLayoutStore(defaults: defaults, accessibilityTrusted: {
            trustChecks += 1
            return false
        })
        let host = NSHostingView(rootView: SavedWindowLayoutsView(store: store))
        host.frame = NSRect(x: 0, y: 0, width: 660, height: 560)
        host.layoutSubtreeIfNeeded()
        _ = host.fittingSize

        XCTAssertTrue(store.layouts.isEmpty)
        do {
            _ = try await store.capture(name: "Test")
            XCTFail("Capture should stop at the injected permission check.")
        } catch {
            XCTAssertEqual(error.localizedDescription,
                           L("Enable Kebloop in System Settings → Privacy & Security → Accessibility to capture or restore window layouts."))
        }
        XCTAssertEqual(trustChecks, 1)
        XCTAssertTrue(store.layouts.isEmpty)
        XCTAssertNil(defaults.data(forKey: "savedWindowLayouts.v1"))
    }
}
