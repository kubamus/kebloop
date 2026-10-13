import AppKit
import ApplicationServices
import SwiftUI

enum WindowMenuSearchScope: Equatable, Sendable {
    case windows
    case menus
}

struct WindowMenuSearchEntry: Identifiable, @unchecked Sendable {
    enum Kind: String {
        case window
        case menuCommand
    }

    let id: String
    let title: String
    let detail: String
    let kind: Kind
    let processIdentifier: pid_t
    fileprivate let element: AXUIElement
}

private struct WindowMenuSearchTarget: Sendable {
    let processIdentifier: pid_t
    let applicationName: String
}

private enum WindowMenuSearchScanner {
    enum Failure: LocalizedError {
        case permission
        case noAccessibleResults

        var errorDescription: String? {
            switch self {
            case .permission:
                return L("Enable Kebloop in System Settings → Privacy & Security → Accessibility to search windows and menus.")
            case .noAccessibleResults:
                return L("No windows or menu commands are currently accessible in this application.")
            }
        }
    }

    static func windows(in targets: [WindowMenuSearchTarget]) throws -> [WindowMenuSearchEntry] {
        guard AXIsProcessTrusted() else { throw Failure.permission }
        let deadline = DispatchTime.now() + .seconds(6)
        var inspected = 0
        var results: [WindowMenuSearchEntry] = []

        applicationLoop: for target in targets.prefix(100) {
            guard DispatchTime.now() < deadline, inspected < 700 else { break }
            let app = AXUIElementCreateApplication(target.processIdentifier)
            AXUIElementSetMessagingTimeout(app, 0.1)
            guard let windows = elements(attribute: kAXWindowsAttribute, of: app) else { continue }
            for (index, window) in windows.prefix(100).enumerated() {
                guard DispatchTime.now() < deadline, inspected < 700 else { break applicationLoop }
                inspected += 1
                AXUIElementSetMessagingTimeout(window, 0.1)
                guard let title = stringAttribute(window, kAXTitleAttribute), !title.isEmpty else { continue }
                results.append(WindowMenuSearchEntry(id: "window:\(target.processIdentifier):\(index):\(title)",
                                                     title: title, detail: target.applicationName,
                                                     kind: .window, processIdentifier: target.processIdentifier,
                                                     element: window))
            }
        }
        guard !results.isEmpty else { throw Failure.noAccessibleResults }
        return results
    }

    static func menus(in target: WindowMenuSearchTarget) throws -> [WindowMenuSearchEntry] {
        guard AXIsProcessTrusted() else { throw Failure.permission }
        let app = AXUIElementCreateApplication(target.processIdentifier)
        AXUIElementSetMessagingTimeout(app, 0.1)
        var results: [WindowMenuSearchEntry] = []
        if let menuBar = element(attribute: kAXMenuBarAttribute, of: app) {
            var budget = ScanBudget(deadline: DispatchTime.now() + .seconds(5), maxNodes: 1_000)
            walkMenu(menuBar, path: [], target: target, depth: 0, budget: &budget, results: &results)
        }
        guard !results.isEmpty else { throw Failure.noAccessibleResults }
        return results
    }

    private struct ScanBudget {
        let deadline: DispatchTime
        let maxNodes: Int
        private(set) var visitedNodes = 0

        mutating func consumeNode() -> Bool {
            guard visitedNodes < maxNodes, DispatchTime.now() < deadline else { return false }
            visitedNodes += 1
            return true
        }

        var hasTime: Bool { DispatchTime.now() < deadline && visitedNodes < maxNodes }
    }

    private static func walkMenu(_ node: AXUIElement, path: [String], target: WindowMenuSearchTarget,
                                 depth: Int, budget: inout ScanBudget, results: inout [WindowMenuSearchEntry]) {
        guard depth <= 16, budget.consumeNode() else { return }
        AXUIElementSetMessagingTimeout(node, 0.1)
        let role = stringAttribute(node, kAXRoleAttribute)
        let title = stringAttribute(node, kAXTitleAttribute)?.trimmingCharacters(in: .whitespacesAndNewlines)
        let children = elements(attribute: kAXChildrenAttribute, of: node) ?? []
        let submenu = element(attribute: "AXMenu", of: node)
        let descendants = children + (submenu.map { [$0] } ?? [])

        if role == kAXMenuBarItemRole, let title, !title.isEmpty {
            for child in descendants where budget.hasTime {
                walkMenu(child, path: path + [title], target: target, depth: depth + 1,
                         budget: &budget, results: &results)
            }
            return
        }
        if role == kAXMenuItemRole, let title, !title.isEmpty {
            if !descendants.isEmpty {
                for child in descendants where budget.hasTime {
                    walkMenu(child, path: path + [title], target: target, depth: depth + 1,
                             budget: &budget, results: &results)
                }
            } else if booleanAttribute(node, kAXEnabledAttribute) {
                let location = path.isEmpty ? L("Menu") : path.joined(separator: " › ")
                results.append(WindowMenuSearchEntry(id: "menu:\(target.processIdentifier):\(budget.visitedNodes):\(title)",
                                                     title: title, detail: "\(target.applicationName) · \(location)",
                                                     kind: .menuCommand, processIdentifier: target.processIdentifier,
                                                     element: node))
            }
            return
        }
        for child in descendants where budget.hasTime {
            walkMenu(child, path: path, target: target, depth: depth + 1,
                     budget: &budget, results: &results)
        }
    }

    private static func element(attribute name: String, of owner: AXUIElement) -> AXUIElement? {
        guard let value = copyAttribute(name, of: owner), CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
        return (value as! AXUIElement)
    }

    private static func elements(attribute name: String, of owner: AXUIElement) -> [AXUIElement]? {
        guard let value = copyAttribute(name, of: owner), let values = value as? [AXUIElement] else { return nil }
        return values
    }

    static func stringAttribute(_ owner: AXUIElement, _ name: String) -> String? {
        copyAttribute(name, of: owner) as? String
    }

    static func booleanAttribute(_ owner: AXUIElement, _ name: String) -> Bool {
        copyAttribute(name, of: owner) as? Bool == true
    }

    private static func copyAttribute(_ name: String, of owner: AXUIElement) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(owner, name as CFString, &value) == .success else { return nil }
        return value
    }
}

@MainActor
final class WindowMenuSearchModel: ObservableObject {
    enum Status: Equatable {
        case idle
        case loading
        case ready
        case failed(String)
    }

    @Published private(set) var entries: [WindowMenuSearchEntry] = []
    @Published private(set) var status: Status = .idle
    private let application: NSRunningApplication?
    private let scope: WindowMenuSearchScope
    private let accessibilityTrusted: () -> Bool

    init(application: NSRunningApplication?, scope: WindowMenuSearchScope,
         accessibilityTrusted: (() -> Bool)? = nil) {
        self.application = application
        self.scope = scope
        self.accessibilityTrusted = accessibilityTrusted ?? { AXIsProcessTrusted() }
    }

    func refresh() async {
        let targets: [WindowMenuSearchTarget]
        switch scope {
        case .windows:
            targets = NSWorkspace.shared.runningApplications
                .filter {
                    !$0.isTerminated && $0.activationPolicy != .prohibited
                        && $0.processIdentifier != ProcessInfo.processInfo.processIdentifier
                }
                .sorted { lhs, rhs in
                    if lhs.isActive != rhs.isActive { return lhs.isActive }
                    return (lhs.localizedName ?? "").localizedCaseInsensitiveCompare(rhs.localizedName ?? "") == .orderedAscending
                }
                .map { WindowMenuSearchTarget(processIdentifier: $0.processIdentifier,
                                              applicationName: $0.localizedName ?? L("Application")) }
        case .menus:
            guard let application, !application.isTerminated,
                  application.processIdentifier != ProcessInfo.processInfo.processIdentifier else {
                status = .failed(L("The previous application is no longer available."))
                return
            }
            targets = [WindowMenuSearchTarget(processIdentifier: application.processIdentifier,
                                              applicationName: application.localizedName ?? L("Application"))]
        }
        guard accessibilityTrusted() else {
            status = .failed(L("Enable Kebloop in System Settings → Privacy & Security → Accessibility to search windows and menus."))
            return
        }

        status = .loading
        do {
            let requestedScope = scope
            let found = try await Task.detached(priority: .userInitiated) {
                switch requestedScope {
                case .windows: try WindowMenuSearchScanner.windows(in: targets)
                case .menus: try WindowMenuSearchScanner.menus(in: targets[0])
                }
            }.value
            entries = found
            status = .ready
        } catch {
            status = .failed(error.localizedDescription)
        }
    }

    func activate(_ entry: WindowMenuSearchEntry) throws {
        guard let application = NSWorkspace.shared.runningApplications.first(where: {
            $0.processIdentifier == entry.processIdentifier && !$0.isTerminated
        }), application.processIdentifier != ProcessInfo.processInfo.processIdentifier else {
            throw SearchFailure.applicationUnavailable
        }
        guard accessibilityTrusted() else { throw SearchFailure.permission }
        guard WindowMenuSearchScanner.stringAttribute(entry.element, kAXTitleAttribute) != nil else {
            throw SearchFailure.entryUnavailable
        }
        if entry.kind == .menuCommand,
           !WindowMenuSearchScanner.booleanAttribute(entry.element, kAXEnabledAttribute) {
            throw SearchFailure.entryUnavailable
        }
        _ = application.activate(options: [.activateAllWindows])
        let action = entry.kind == .window ? kAXRaiseAction : kAXPressAction
        let result = AXUIElementPerformAction(entry.element, action as CFString)
        guard result == .success else { throw SearchFailure.accessibility(result) }
    }

    func contains(_ entry: WindowMenuSearchEntry, query: String) -> Bool {
        guard !query.isEmpty else { return true }
        return entry.title.localizedCaseInsensitiveContains(query)
            || entry.detail.localizedCaseInsensitiveContains(query)
    }

    static func requestAccessibilityPermission() {
        AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary)
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") {
            NSWorkspace.shared.open(url)
        }
    }

    private enum SearchFailure: LocalizedError {
        case permission
        case applicationUnavailable
        case entryUnavailable
        case accessibility(AXError)

        var errorDescription: String? {
            switch self {
            case .permission:
                return L("Enable Kebloop in System Settings → Privacy & Security → Accessibility to use this result.")
            case .applicationUnavailable:
                return L("The application is no longer available.")
            case .entryUnavailable:
                return L("That window or menu command disappeared or is disabled. Search again to refresh the results.")
            case .accessibility(let code):
                return L("The result could not be activated (Accessibility error \(code.rawValue)). Search again to refresh the results.")
            }
        }
    }
}

@MainActor
struct WindowMenuSearchView: View {
    @StateObject private var model: WindowMenuSearchModel
    @State private var query = ""
    @State private var selectedID: String?
    @State private var feedback: String?
    private let onActivate: () -> Void
    private let scope: WindowMenuSearchScope
    private let onRequestAccessibilityPermission: @MainActor () -> Void

    init(application: NSRunningApplication?, scope: WindowMenuSearchScope,
         onActivate: @escaping () -> Void = {},
         onRequestAccessibilityPermission: (@MainActor () -> Void)? = nil) {
        _model = StateObject(wrappedValue: WindowMenuSearchModel(application: application, scope: scope))
        self.onActivate = onActivate
        self.scope = scope
        self.onRequestAccessibilityPermission = onRequestAccessibilityPermission ?? { WindowMenuSearchModel.requestAccessibilityPermission() }
    }

    init(model: WindowMenuSearchModel, scope: WindowMenuSearchScope,
         onActivate: @escaping () -> Void = {},
         onRequestAccessibilityPermission: (@MainActor () -> Void)? = nil) {
        _model = StateObject(wrappedValue: model)
        self.onActivate = onActivate
        self.scope = scope
        self.onRequestAccessibilityPermission = onRequestAccessibilityPermission ?? { WindowMenuSearchModel.requestAccessibilityPermission() }
    }

    private var visibleEntries: [WindowMenuSearchEntry] {
        model.entries.filter { model.contains($0, query: query) }
    }

    var body: some View {
        VStack(spacing: 0) {
            TextField(scope == .windows ? L("Search open window titles") : L("Search previous app menu commands"), text: $query)
                .textFieldStyle(.roundedBorder)
                .onSubmit(activateSelected)
                .padding(12)
            if case .loading = model.status {
                ProgressView(L("Searching…"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if case .failed(let message) = model.status {
                ContentUnavailableView {
                    Label(L("Window & Menu Search"), systemImage: "macwindow.on.rectangle")
                } description: {
                    Text(message)
                } actions: {
                    Button(action: onRequestAccessibilityPermission) {
                        Text(L("Open Accessibility Settings"))
                    }
                    Button(L("Try Again")) { Task { await model.refresh() } }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(visibleEntries, selection: $selectedID) { entry in
                    Button {
                        selectedID = entry.id
                        activate(entry)
                    } label: {
                        HStack(spacing: 10) {
                            Image(systemName: entry.kind == .window ? "macwindow" : "text.alignleft")
                                .foregroundStyle(.secondary)
                                .frame(width: 20)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(entry.title).lineLimit(1)
                                Text(entry.detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }
                            Spacer()
                            if entry.kind == .menuCommand {
                                Image(systemName: "return").font(.caption).foregroundStyle(.tertiary)
                            }
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .tag(entry.id)
                }
                .overlay {
                    if visibleEntries.isEmpty { ContentUnavailableView.search(text: query) }
                }
            }
            if !visibleEntries.isEmpty {
                HStack {
                    Spacer()
                    Button(L("Open Selected Result"), action: activateSelected)
                        .keyboardShortcut(.return, modifiers: [])
                }.padding(10)
            }
            if let feedback {
                HStack {
                    Text(feedback).font(.caption).foregroundStyle(.red)
                    Spacer()
                    Button(L("Dismiss")) { self.feedback = nil }.buttonStyle(.borderless)
                }
                .padding(10)
                .accessibilityAddTraits(.updatesFrequently)
            }
        }
        .frame(minWidth: 440, minHeight: 380)
        .task { if model.status == .idle { await model.refresh() } }
        .onChange(of: visibleEntries.map(\.id)) { _, ids in
            if !ids.contains(selectedID ?? "") { selectedID = ids.first }
        }
    }

    private func activateSelected() {
        guard let entry = visibleEntries.first(where: { $0.id == selectedID }) ?? visibleEntries.first else { return }
        activate(entry)
    }

    private func activate(_ entry: WindowMenuSearchEntry) {
        do { try model.activate(entry); onActivate() }
        catch {
            feedback = error.localizedDescription
            Task { await model.refresh() }
        }
    }

}

@MainActor
final class WindowMenuSearchWindowController {
    static let shared = WindowMenuSearchWindowController()
    private var window: NSWindow?

    func show(application: NSRunningApplication?, scope: WindowMenuSearchScope) {
        let targetWindow = window ?? NSWindow(contentRect: NSRect(x: 0, y: 0, width: 560, height: 560),
                                              styleMask: [.titled, .closable, .miniaturizable, .resizable],
                                              backing: .buffered, defer: false)
        targetWindow.isReleasedWhenClosed = false
        targetWindow.title = scope == .windows ? L("Window Search") : L("Menu Search")
        targetWindow.contentView = NSHostingView(rootView: WindowMenuSearchView(
            application: application, scope: scope,
            onActivate: { [weak targetWindow] in targetWindow?.close() }))
        targetWindow.center()
        targetWindow.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        window = targetWindow
    }
}
