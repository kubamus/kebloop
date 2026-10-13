import AppKit
import ApplicationServices
import SwiftUI

struct SavedLayoutDisplay: Codable, Equatable, Sendable {
    let identifier: UInt32?
    let name: String
    let width: Double
    let height: Double

    var aspectRatio: Double { width > 0 ? width / max(height, 1) : 1 }
}

struct LayoutRect: Codable, Equatable, Sendable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double

    init(_ rect: CGRect) {
        x = rect.origin.x
        y = rect.origin.y
        width = rect.width
        height = rect.height
    }

    var cgRect: CGRect { CGRect(x: x, y: y, width: width, height: height) }
}

struct SavedWindowFrame: Codable, Equatable, Sendable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double

    init(frame: CGRect, within display: CGRect) {
        x = (frame.minX - display.minX) / max(display.width, 1)
        y = (frame.minY - display.minY) / max(display.height, 1)
        width = frame.width / max(display.width, 1)
        height = frame.height / max(display.height, 1)
    }

    func frame(within display: CGRect) -> CGRect {
        CGRect(x: display.minX + x * display.width,
               y: display.minY + y * display.height,
               width: width * display.width,
               height: height * display.height)
    }

    static func clamp(_ frame: CGRect, to display: CGRect) -> CGRect {
        let width = min(max(frame.width, 1), display.width)
        let height = min(max(frame.height, 1), display.height)
        return CGRect(x: min(max(frame.minX, display.minX), display.maxX - width),
                      y: min(max(frame.minY, display.minY), display.maxY - height),
                      width: width, height: height)
    }
}

struct SavedWindowPlacement: Codable, Equatable, Identifiable, Sendable {
    var id: String { "\(bundleIdentifier):\(title):\(ordinal)" }
    let bundleIdentifier: String
    let applicationName: String
    let title: String
    let ordinal: Int
    let display: SavedLayoutDisplay
    let frame: SavedWindowFrame
}

private struct LayoutCaptureApplication: Sendable {
    let processIdentifier: pid_t
    let bundleIdentifier: String
    let name: String
}

private struct LayoutCurrentDisplay: Sendable {
    let descriptor: SavedLayoutDisplay
    let visibleBounds: LayoutRect
}

private struct LayoutCaptureOutcome: Sendable {
    let placements: [SavedWindowPlacement]
    let skippedWindowCount: Int
}

enum SavedWindowLayoutMatching {
    static func displayIndex(for saved: SavedLayoutDisplay, among displays: [SavedLayoutDisplay]) -> Int? {
        guard !displays.isEmpty else { return nil }
        if let identifier = saved.identifier,
           let exact = displays.firstIndex(where: { $0.identifier == identifier }) { return exact }
        if !saved.name.isEmpty, let named = displays.firstIndex(where: { $0.name == saved.name }) { return named }
        return displays.indices.min { lhs, rhs in
            displayDistance(saved, displays[lhs]) < displayDistance(saved, displays[rhs])
        }
    }

    static func matchWindowIndex(title: String, ordinal: Int, titles: [String], usedIndexes: Set<Int> = []) -> Int? {
        let exact = titles.indices.filter { titles[$0] == title && !usedIndexes.contains($0) }
        guard !exact.isEmpty else { return nil }
        let matchingPositions = titles.indices.filter { titles[$0] == title }
        if ordinal >= 0, ordinal < matchingPositions.count {
            let preferred = matchingPositions[ordinal]
            if !usedIndexes.contains(preferred) { return preferred }
        }
        return exact.first
    }

    private static func displayDistance(_ lhs: SavedLayoutDisplay, _ rhs: SavedLayoutDisplay) -> Double {
        let aspect = abs(log(max(lhs.aspectRatio, 0.1) / max(rhs.aspectRatio, 0.1)))
        let size = abs(log(max(lhs.width, 1) / max(rhs.width, 1)))
            + abs(log(max(lhs.height, 1) / max(rhs.height, 1)))
        return aspect * 2 + size * 0.25
    }
}

private enum SavedLayoutCaptureWorker {
    enum Failure: LocalizedError {
        case permission
        var errorDescription: String? {
            L("Enable Kebloop in System Settings → Privacy & Security → Accessibility to capture window layouts.")
        }
    }

    static func capture(applications: [LayoutCaptureApplication], displays: [LayoutCurrentDisplay]) throws -> LayoutCaptureOutcome {
        guard AXIsProcessTrusted() else { throw Failure.permission }
        let deadline = DispatchTime.now() + .seconds(4)
        var placements: [SavedWindowPlacement] = []
        var skipped = 0
        var inspected = 0

        applicationLoop: for application in applications.prefix(100) {
            guard DispatchTime.now() < deadline, inspected < 1_000 else { break }
            let appElement = AXUIElementCreateApplication(application.processIdentifier)
            AXUIElementSetMessagingTimeout(appElement, 0.15)
            guard let windows = elements(attribute: kAXWindowsAttribute, of: appElement) else { continue }
            var titleOccurrences: [String: Int] = [:]

            for window in windows.prefix(100) {
                guard DispatchTime.now() < deadline, inspected < 1_000 else { break applicationLoop }
                inspected += 1
                AXUIElementSetMessagingTimeout(window, 0.15)
                guard let title = stringAttribute(window, kAXTitleAttribute), !title.isEmpty else { skipped += 1; continue }
                let ordinal = titleOccurrences[title, default: 0]
                titleOccurrences[title] = ordinal + 1
                guard stringAttribute(window, kAXSubroleAttribute) == kAXStandardWindowSubrole,
                      !boolean(window, kAXMinimizedAttribute), !boolean(window, "AXFullScreen"),
                      ((elements(attribute: "AXSheets", of: window) ?? []).isEmpty),
                      isSettable(window, kAXPositionAttribute), isSettable(window, kAXSizeAttribute),
                      let frame = readFrame(window),
                      let screenIndex = screenIndex(for: frame, screens: displays.map { $0.visibleBounds.cgRect }) else {
                    skipped += 1
                    continue
                }
                let display = displays[screenIndex]
                placements.append(SavedWindowPlacement(bundleIdentifier: application.bundleIdentifier,
                                                       applicationName: application.name, title: title, ordinal: ordinal,
                                                       display: display.descriptor,
                                                       frame: SavedWindowFrame(frame: frame, within: display.visibleBounds.cgRect)))
            }
        }
        return LayoutCaptureOutcome(placements: placements, skippedWindowCount: skipped)
    }

    private static func screenIndex(for frame: CGRect, screens: [CGRect]) -> Int? {
        guard !screens.isEmpty else { return nil }
        return screens.indices.max { lhs, rhs in
            let first = frame.intersection(screens[lhs]), second = frame.intersection(screens[rhs])
            let firstArea = first.isNull ? 0 : first.width * first.height
            let secondArea = second.isNull ? 0 : second.width * second.height
            return firstArea < secondArea
        }
    }

    private static func readFrame(_ window: AXUIElement) -> CGRect? {
        guard let position = copyAttribute(kAXPositionAttribute, of: window),
              let size = copyAttribute(kAXSizeAttribute, of: window),
              CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { return nil }
        var point = CGPoint.zero
        var dimensions = CGSize.zero
        guard AXValueGetValue(position as! AXValue, .cgPoint, &point), AXValueGetValue(size as! AXValue, .cgSize, &dimensions),
              dimensions.width > 0, dimensions.height > 0 else { return nil }
        return CGRect(origin: point, size: dimensions)
    }

    private static func isSettable(_ element: AXUIElement, _ name: String) -> Bool {
        var settable = DarwinBoolean(false)
        return AXUIElementIsAttributeSettable(element, name as CFString, &settable) == .success && settable.boolValue
    }

    private static func boolean(_ element: AXUIElement, _ name: String) -> Bool {
        copyAttribute(name, of: element) as? Bool == true
    }

    private static func elements(attribute name: String, of owner: AXUIElement) -> [AXUIElement]? {
        guard let value = copyAttribute(name, of: owner) else { return nil }
        return value as? [AXUIElement]
    }

    private static func stringAttribute(_ owner: AXUIElement, _ name: String) -> String? {
        copyAttribute(name, of: owner) as? String
    }

    private static func copyAttribute(_ name: String, of owner: AXUIElement) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(owner, name as CFString, &value) == .success else { return nil }
        return value
    }
}

private enum SavedLayoutRestoreWorker {
    private struct Window {
        let title: String
        let element: AXUIElement
    }

    private enum Failure: LocalizedError {
        case permission
        var errorDescription: String? {
            L("Enable Kebloop in System Settings → Privacy & Security → Accessibility to restore window layouts.")
        }
    }

    static func restore(_ layout: SavedWindowLayout, applications: [LayoutCaptureApplication],
                        displays: [LayoutCurrentDisplay]) throws -> SavedLayoutRestoreReport {
        guard AXIsProcessTrusted() else { throw Failure.permission }
        let deadline = DispatchTime.now() + .seconds(6)
        let applicationByIdentifier = Dictionary(applications.map { ($0.bundleIdentifier, $0) }, uniquingKeysWith: { first, _ in first })
        let displayDescriptors = displays.map(\.descriptor)
        var restored = 0
        var missingApplications = Set<String>()
        var missingWindows: [String] = []
        var failures: [String] = []
        var cachedWindows: [String: [Window]] = [:]
        var usedWindowIndexes: [String: Set<Int>] = [:]
        let maximumWindows = 500
        let placements = Array(layout.windows.prefix(maximumWindows))

        for placement in placements {
            let label = "\(placement.applicationName) — \(placement.title)"
            guard DispatchTime.now() < deadline else {
                failures.append(String(format: L("Restore stopped at its time limit before %@."), label))
                break
            }
            guard isValid(placement) else {
                failures.append("\(label): " + L("saved window geometry is invalid"))
                continue
            }
            guard let application = applicationByIdentifier[placement.bundleIdentifier] else {
                missingApplications.insert(placement.applicationName)
                continue
            }
            let identifier = placement.bundleIdentifier
            if cachedWindows[identifier] == nil {
                let app = AXUIElementCreateApplication(application.processIdentifier)
                AXUIElementSetMessagingTimeout(app, 0.1)
                cachedWindows[identifier] = windows(of: app)
            }
            guard let candidates = cachedWindows[identifier],
                  let index = SavedWindowLayoutMatching.matchWindowIndex(
                    title: placement.title, ordinal: placement.ordinal,
                    titles: candidates.map(\.title), usedIndexes: usedWindowIndexes[identifier] ?? []) else {
                missingWindows.append(label)
                continue
            }
            usedWindowIndexes[identifier, default: []].insert(index)
            guard let displayIndex = SavedWindowLayoutMatching.displayIndex(for: placement.display, among: displayDescriptors) else {
                failures.append("\(label): " + L("display unavailable"))
                continue
            }
            let display = displays[displayIndex].visibleBounds.cgRect
            let target = SavedWindowFrame.clamp(placement.frame.frame(within: display), to: display)
            do {
                try setFrame(target, on: candidates[index].element)
                restored += 1
            } catch {
                failures.append("\(label): \(error.localizedDescription)")
            }
        }
        if layout.windows.count > maximumWindows {
            failures.append(String(format: L("Restore was limited to %d windows."), maximumWindows))
        }
        return SavedLayoutRestoreReport(restoredWindowCount: restored,
                                        missingApplications: missingApplications.sorted(),
                                        missingWindows: missingWindows, failures: failures)
    }

    private static func isValid(_ placement: SavedWindowPlacement) -> Bool {
        let frame = placement.frame
        let display = placement.display
        return !placement.bundleIdentifier.isEmpty && !placement.title.isEmpty && placement.ordinal >= 0
            && display.width.isFinite && display.height.isFinite && display.width > 0 && display.height > 0
            && frame.x.isFinite && frame.y.isFinite && frame.width.isFinite && frame.height.isFinite
            && frame.width > 0 && frame.height > 0
    }

    private static func windows(of app: AXUIElement) -> [Window] {
        guard let values = elements(attribute: kAXWindowsAttribute, of: app) else { return [] }
        return values.prefix(100).compactMap { window in
            AXUIElementSetMessagingTimeout(window, 0.1)
            guard let title = stringAttribute(window, kAXTitleAttribute), !title.isEmpty else { return nil }
            return Window(title: title, element: window)
        }
    }

    private static func setFrame(_ frame: CGRect, on window: AXUIElement) throws {
        AXUIElementSetMessagingTimeout(window, 0.1)
        guard stringAttribute(window, kAXSubroleAttribute) == kAXStandardWindowSubrole,
              !boolean(window, kAXMinimizedAttribute), !boolean(window, "AXFullScreen"),
              ((elements(attribute: "AXSheets", of: window) ?? []).isEmpty),
              isSettable(window, kAXPositionAttribute), isSettable(window, kAXSizeAttribute),
              let original = readFrame(window) else { throw LayoutWindowFailure.unsupported }
        var size = frame.size
        var point = frame.origin
        guard let sizeValue = AXValueCreate(.cgSize, &size), let pointValue = AXValueCreate(.cgPoint, &point) else {
            throw LayoutWindowFailure.unsupported
        }
        let sizeResult = AXUIElementSetAttributeValue(window, kAXSizeAttribute as CFString, sizeValue)
        guard sizeResult == .success else { throw LayoutWindowFailure.accessibility(sizeResult) }
        let positionResult = AXUIElementSetAttributeValue(window, kAXPositionAttribute as CFString, pointValue)
        guard positionResult == .success else {
            try? setRawFrame(original, on: window)
            throw LayoutWindowFailure.accessibility(positionResult)
        }
    }

    private static func setRawFrame(_ frame: CGRect, on window: AXUIElement) throws {
        var size = frame.size
        var point = frame.origin
        guard let sizeValue = AXValueCreate(.cgSize, &size), let pointValue = AXValueCreate(.cgPoint, &point) else {
            throw LayoutWindowFailure.unsupported
        }
        let sizeResult = AXUIElementSetAttributeValue(window, kAXSizeAttribute as CFString, sizeValue)
        guard sizeResult == .success else { throw LayoutWindowFailure.accessibility(sizeResult) }
        let positionResult = AXUIElementSetAttributeValue(window, kAXPositionAttribute as CFString, pointValue)
        guard positionResult == .success else { throw LayoutWindowFailure.accessibility(positionResult) }
    }

    private enum LayoutWindowFailure: LocalizedError {
        case unsupported
        case accessibility(AXError)

        var errorDescription: String? {
            switch self {
            case .unsupported: return L("window is no longer movable or is minimized, full-screen, or a sheet")
            case .accessibility(let code): return String(format: L("Accessibility error %d"), code.rawValue)
            }
        }
    }

    private static func readFrame(_ window: AXUIElement) -> CGRect? {
        guard let position = copyAttribute(kAXPositionAttribute, of: window),
              let size = copyAttribute(kAXSizeAttribute, of: window),
              CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { return nil }
        var point = CGPoint.zero
        var dimensions = CGSize.zero
        guard AXValueGetValue(position as! AXValue, .cgPoint, &point), AXValueGetValue(size as! AXValue, .cgSize, &dimensions),
              dimensions.width > 0, dimensions.height > 0 else { return nil }
        return CGRect(origin: point, size: dimensions)
    }

    private static func isSettable(_ element: AXUIElement, _ name: String) -> Bool {
        var settable = DarwinBoolean(false)
        return AXUIElementIsAttributeSettable(element, name as CFString, &settable) == .success && settable.boolValue
    }

    private static func boolean(_ element: AXUIElement, _ name: String) -> Bool {
        copyAttribute(name, of: element) as? Bool == true
    }

    private static func elements(attribute name: String, of owner: AXUIElement) -> [AXUIElement]? {
        guard let value = copyAttribute(name, of: owner) else { return nil }
        return value as? [AXUIElement]
    }

    private static func stringAttribute(_ owner: AXUIElement, _ name: String) -> String? {
        copyAttribute(name, of: owner) as? String
    }

    private static func copyAttribute(_ name: String, of owner: AXUIElement) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(owner, name as CFString, &value) == .success else { return nil }
        return value
    }
}

struct SavedWindowLayout: Codable, Equatable, Identifiable, Sendable {
    let id: String
    var name: String
    let createdAt: Date
    var windows: [SavedWindowPlacement]

    init(id: String = UUID().uuidString, name: String, createdAt: Date = Date(), windows: [SavedWindowPlacement]) {
        self.id = id
        self.name = name
        self.createdAt = createdAt
        self.windows = windows
    }
}

struct SavedLayoutCaptureResult: Sendable {
    let layout: SavedWindowLayout
    let skippedWindowCount: Int
}

struct SavedLayoutRestoreReport: Sendable {
    let restoredWindowCount: Int
    let missingApplications: [String]
    let missingWindows: [String]
    let failures: [String]

    var summary: String {
        var parts = [String(format: L("Restored %d windows."), restoredWindowCount)]
        if !missingApplications.isEmpty {
            parts.append(String(format: L("Apps not running: %@."), missingApplications.joined(separator: ", ")))
        }
        if !missingWindows.isEmpty {
            parts.append(String(format: L("Windows not found: %@."), missingWindows.joined(separator: ", ")))
        }
        if !failures.isEmpty {
            parts.append(String(format: L("Some windows could not be restored: %@."), failures.joined(separator: "; ")))
        }
        return parts.joined(separator: " ")
    }
}

@MainActor
final class SavedWindowLayoutStore: ObservableObject {
    enum Failure: LocalizedError {
        case permission
        case noWindows
        case noDisplays
        case corruptLayout

        var errorDescription: String? {
            switch self {
            case .permission:
                return L("Enable Kebloop in System Settings → Privacy & Security → Accessibility to capture or restore window layouts.")
            case .noWindows:
                return L("No movable windows were found in other running applications.")
            case .noDisplays:
                return L("No display is available to restore this layout.")
            case .corruptLayout:
                return L("This saved layout is incomplete and cannot be restored.")
            }
        }
    }

    @Published private(set) var layouts: [SavedWindowLayout]
    private let defaults: UserDefaults
    private let accessibilityTrusted: () -> Bool
    private let storageKey = "savedWindowLayouts.v1"
    private static let maximumLayouts = 40

    init(defaults: UserDefaults = .standard, accessibilityTrusted: (() -> Bool)? = nil) {
        self.defaults = defaults
        self.accessibilityTrusted = accessibilityTrusted ?? { AXIsProcessTrusted() }
        layouts = defaults.data(forKey: storageKey).flatMap { try? JSONDecoder().decode([SavedWindowLayout].self, from: $0) } ?? []
    }

    func capture(name: String) async throws -> SavedLayoutCaptureResult {
        guard accessibilityTrusted() else { throw Failure.permission }
        let screenSnapshots = Self.currentDisplays()
        guard !screenSnapshots.isEmpty else { throw Failure.noDisplays }

        let ownBundleIdentifier = Bundle.main.bundleIdentifier
        let applications = NSWorkspace.shared.runningApplications.compactMap { app -> LayoutCaptureApplication? in
            guard !app.isTerminated, app.activationPolicy != .prohibited,
                  app.processIdentifier != ProcessInfo.processInfo.processIdentifier,
                  let bundleIdentifier = app.bundleIdentifier, bundleIdentifier != ownBundleIdentifier else { return nil }
            return LayoutCaptureApplication(processIdentifier: app.processIdentifier,
                                            bundleIdentifier: bundleIdentifier,
                                            name: app.localizedName ?? bundleIdentifier)
        }
        let outcome = try await Task.detached(priority: .userInitiated) {
            try SavedLayoutCaptureWorker.capture(applications: applications, displays: screenSnapshots)
        }.value
        guard !outcome.placements.isEmpty else { throw Failure.noWindows }

        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let layout = SavedWindowLayout(name: trimmedName.isEmpty ? L("Window Layout") : trimmedName,
                                       windows: outcome.placements)
        layouts.insert(layout, at: 0)
        if layouts.count > Self.maximumLayouts { layouts.removeLast(layouts.count - Self.maximumLayouts) }
        persist()
        return SavedLayoutCaptureResult(layout: layout, skippedWindowCount: outcome.skippedWindowCount)
    }

    func restore(_ layout: SavedWindowLayout) async throws -> SavedLayoutRestoreReport {
        guard accessibilityTrusted() else { throw Failure.permission }
        guard !layout.windows.isEmpty else { throw Failure.corruptLayout }
        let screens = Self.currentDisplays()
        guard !screens.isEmpty else { throw Failure.noDisplays }
        let ownBundleIdentifier = Bundle.main.bundleIdentifier
        let applications = NSWorkspace.shared.runningApplications.compactMap { app -> LayoutCaptureApplication? in
            guard !app.isTerminated, app.activationPolicy != .prohibited,
                  app.processIdentifier != ProcessInfo.processInfo.processIdentifier,
                  let bundleIdentifier = app.bundleIdentifier, bundleIdentifier != ownBundleIdentifier else { return nil }
            return LayoutCaptureApplication(processIdentifier: app.processIdentifier,
                                            bundleIdentifier: bundleIdentifier,
                                            name: app.localizedName ?? bundleIdentifier)
        }
        return try await Task.detached(priority: .userInitiated) {
            try SavedLayoutRestoreWorker.restore(layout, applications: applications, displays: screens)
        }.value
    }

    func delete(id: String) {
        layouts.removeAll { $0.id == id }
        persist()
    }

    func rename(id: String, to name: String) {
        guard let index = layouts.firstIndex(where: { $0.id == id }) else { return }
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        layouts[index].name = trimmed
        persist()
    }

    private func persist() {
        guard let data = try? JSONEncoder().encode(layouts) else { return }
        defaults.set(data, forKey: storageKey)
    }

    private static func currentDisplays() -> [LayoutCurrentDisplay] {
        guard let primary = NSScreen.screens.first else { return [] }
        let primaryHeight = primary.frame.maxY
        return NSScreen.screens.map { screen in
            let visible = WindowManagementService.accessibilityRect(screen.visibleFrame, primaryHeight: primaryHeight)
            let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber
            return LayoutCurrentDisplay(
                descriptor: SavedLayoutDisplay(identifier: number?.uint32Value,
                                               name: screen.localizedName,
                                               width: Double(visible.width), height: Double(visible.height)),
                visibleBounds: LayoutRect(visible))
        }
    }

}

@MainActor
struct SavedWindowLayoutsView: View {
    @StateObject private var store: SavedWindowLayoutStore
    @State private var layoutName = ""
    @State private var feedback: String?
    @State private var busy = false

    init(store: SavedWindowLayoutStore? = nil) {
        _store = StateObject(wrappedValue: store ?? SavedWindowLayoutStore())
    }

    var body: some View {
        Form {
            Section {
                Text(L("Save the open windows of running apps and restore them together. Missing apps or documents are skipped and reported."))
                    .foregroundStyle(.secondary)
                HStack {
                    TextField(L("Layout name"), text: $layoutName)
                    Button(L("Capture Layout")) { capture() }
                        .disabled(busy)
                }
            } header: {
                Text(L("Capture"))
            }

            Section(L("Saved Layouts")) {
                if store.layouts.isEmpty {
                    Text(L("No saved layouts yet.")).foregroundStyle(.secondary)
                }
                ForEach(store.layouts) { layout in
                    HStack {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(layout.name)
                            Text(String(format: L("%d windows · %@"), layout.windows.count,
                                        DateFormatter.localizedString(from: layout.createdAt, dateStyle: .medium, timeStyle: .short)))
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button(L("Restore")) { restore(layout) }.disabled(busy)
                        Button(role: .destructive) { store.delete(id: layout.id) } label: {
                            Image(systemName: "trash")
                        }
                        .buttonStyle(.borderless)
                        .accessibilityLabel(String(format: L("Delete %@"), layout.name))
                    }
                }
            }

            if let feedback {
                Section {
                    Text(feedback).font(.callout).accessibilityAddTraits(.updatesFrequently)
                }
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 520, minHeight: 400)
    }

    private func capture() {
        busy = true
        Task {
            defer { busy = false }
            do {
                let result = try await store.capture(name: layoutName)
                layoutName = ""
                feedback = result.skippedWindowCount == 0
                    ? String(format: L("Saved %@ with %d windows."), result.layout.name, result.layout.windows.count)
                    : String(format: L("Saved %@ with %d windows; skipped %d unsupported or unavailable windows."),
                              result.layout.name, result.layout.windows.count, result.skippedWindowCount)
            } catch {
                feedback = error.localizedDescription
            }
        }
    }

    private func restore(_ layout: SavedWindowLayout) {
        busy = true
        Task {
            defer { busy = false }
            do {
                let report = try await store.restore(layout)
                feedback = report.summary
            } catch {
                feedback = error.localizedDescription
            }
        }
    }
}

@MainActor
final class SavedWindowLayoutsWindowController {
    static let shared = SavedWindowLayoutsWindowController()
    private var window: NSWindow?
    private let store = SavedWindowLayoutStore()

    func show() {
        if let window, window.isVisible {
            window.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 660, height: 560),
                              styleMask: [.titled, .closable, .miniaturizable, .resizable],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.title = L("Saved Window Layouts")
        window.contentView = NSHostingView(rootView: SavedWindowLayoutsView(store: store))
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        self.window = window
    }
}
