import AppKit
import SwiftUI

struct ActionMenuItem: Identifiable {
    let action: ResultAction
    let title: String
    var id: String { action.rawValue }
    var symbol: String {
        switch action {
        case .clearCalculationHistory: "trash"
        case .editNote: "square.and.pencil"
        case .copyMarkdown: "doc.on.doc"
        case .exportNote: "square.and.arrow.up"
        case .deleteNote: "trash"
        case .copy: "doc.on.doc"
        case .pastePlainText: "doc.plaintext"
        case .quitApplication: "xmark.circle"
        case .forceQuitApplication: "exclamationmark.octagon"
        case .restartApplication: "arrow.clockwise"
        case .hideApplication: "eye.slash"
        case .removeClipboardItem: "trash"
        case .open: "arrow.up.forward"
        case .reveal: "folder"
        case .copyPath: "doc.on.doc"
        case .openWith: "app.badge"
        case .editQuicklink: "pencil"
        case .pin: "pin"
        case .unpin: "pin.slash"
        case .runWithInput: "text.cursor"
        }
    }
}

private final class ActionPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

@MainActor
final class ActionMenuController: NSObject, ObservableObject, NSWindowDelegate {
    @Published private(set) var selectedIndex = 0
    private(set) var items: [ActionMenuItem] = []
    private var panel: ActionPanel?
    private var onChoose: ((ResultAction) -> Void)?
    private var onClose: ((Bool) -> Void)?
    var isVisible: Bool { panel?.isVisible == true }
    var isKeyWindow: Bool { panel?.isKeyWindow == true }

    func show(items: [ActionMenuItem], title: String, parent: NSWindow, opaque: Bool,
              onChoose: @escaping (ResultAction) -> Void, onClose: @escaping (Bool) -> Void) {
        close(restoreFocus: false)
        guard !items.isEmpty else { return }
        self.items = items; selectedIndex = 0
        self.onChoose = onChoose; self.onClose = onClose
        let size = NSSize(width: 310, height: CGFloat(92 + items.count * 40))
        let menu = ActionPanel(contentRect: NSRect(origin: .zero, size: size), styleMask: [.borderless], backing: .buffered, defer: false)
        menu.isReleasedWhenClosed = false
        menu.backgroundColor = .clear; menu.isOpaque = false; menu.hasShadow = true
        menu.level = parent.level; menu.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        menu.appearance = parent.appearance; menu.delegate = self
        menu.setAccessibilityLabel(L("Result Actions"))
        menu.contentView = NSHostingView(rootView: ActionMenuView(controller: self, resultTitle: title, opaque: opaque))
        let area = parent.screen?.visibleFrame ?? parent.frame
        let origin = NSPoint(x: min(max(parent.frame.maxX - size.width - 10, area.minX), area.maxX - size.width),
                             y: min(max(parent.frame.maxY - size.height - 68, area.minY), area.maxY - size.height))
        menu.setFrameOrigin(origin)
        panel = menu
        parent.addChildWindow(menu, ordered: .above)
        menu.makeKeyAndOrderFront(nil)
        announceSelection()
    }

    func close(restoreFocus: Bool = true) {
        guard let menu = panel else { return }
        panel = nil
        menu.delegate = nil
        menu.parent?.removeChildWindow(menu); menu.orderOut(nil)
        onChoose = nil
        let completion = onClose; onClose = nil
        completion?(restoreFocus)
    }
    func windowDidResignKey(_ notification: Notification) { close(restoreFocus: false) }
    func select(_ index: Int) {
        guard items.indices.contains(index), index != selectedIndex else { return }
        selectedIndex = index; announceSelection()
    }
    func choose(_ index: Int) {
        guard items.indices.contains(index) else { return }
        let action = items[index].action, completion = onChoose
        close()
        completion?(action)
    }
    @discardableResult
    func handleKey(_ event: NSEvent) -> Bool {
        guard isVisible else { return false }
        if LauncherKeyboard.isActionShortcut(event) { close(); return true }
        if LauncherKeyboard.isNoteDeleteShortcut(event),
           let index = items.firstIndex(where: { $0.action == .deleteNote }) {
            choose(index)
            return true
        }
        switch event.keyCode {
        case 53: close()
        case 125: select(min(selectedIndex + 1, items.count - 1))
        case 126: select(max(selectedIndex - 1, 0))
        case 36, 76: choose(selectedIndex)
        default: return false
        }
        return true
    }
    private func announceSelection() {
        guard let panel, items.indices.contains(selectedIndex) else { return }
        NSAccessibility.post(element: panel, notification: .announcementRequested, userInfo: [
            .announcement: items[selectedIndex].title, .priority: NSAccessibilityPriorityLevel.medium.rawValue
        ])
    }
}

/// Ignore hardware-specific flags, Caps Lock, and Function when recognizing a command chord.
enum LauncherKeyboard {
    static func isNoteDeleteShortcut(_ event: NSEvent) -> Bool {
        event.modifierFlags.intersection([.command, .option, .control, .shift]) == .command
            && [51, 117].contains(event.keyCode)
    }

    static func isActionShortcut(_ event: NSEvent) -> Bool {
        event.modifierFlags.intersection([.command, .option, .control, .shift]) == .command
            && event.charactersIgnoringModifiers?.lowercased() == "k"
    }
}

private struct ActionMenuView: View {
    @ObservedObject var controller: ActionMenuController
    let resultTitle: String
    let opaque: Bool
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.colorSchemeContrast) private var contrast
    @FocusState private var focusedAction: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 3) {
                Text(L("Actions")).font(.system(size: 13, weight: .semibold))
                Text(resultTitle).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
            }.padding(.horizontal, 14).padding(.vertical, 12)
            Divider().padding(.horizontal, 8)
            VStack(spacing: 2) {
                ForEach(Array(controller.items.enumerated()), id: \.element.id) { index, item in
                    Button(role: [.removeClipboardItem, .deleteNote].contains(item.action) ? .destructive : nil) { controller.choose(index) } label: {
                        HStack(spacing: 10) {
                            Image(systemName: item.symbol).frame(width: 18).foregroundStyle(.secondary)
                            Text(item.title).font(.system(size: 13, weight: controller.selectedIndex == index ? .medium : .regular))
                            Spacer(minLength: 4)
                            if item.action == .deleteNote {
                                Text("⌘⌫").font(.system(size: 12)).foregroundStyle(.secondary)
                            }
                            if controller.selectedIndex == index { Text(L("↵")).font(.system(size: 12)).foregroundStyle(.secondary) }
                        }.padding(.horizontal, 10).frame(height: 38)
                            .contentShape(Rectangle())
                            .background(controller.selectedIndex == index ? Color.primary.opacity(contrast == .increased ? 0.2 : 0.09) : .clear, in: RoundedRectangle(cornerRadius: 7))
                    }.buttonStyle(.plain)
                        .focusable()
                        .focused($focusedAction, equals: item.id)
                        .focusEffectDisabled()
                        .onHover { hovering in if hovering { controller.select(index) } }
                        .accessibilityLabel(item.title)
                        .accessibilityAddTraits(controller.selectedIndex == index ? .isSelected : [])
                }
            }.padding(6)
            Divider().padding(.horizontal, 8)
            HStack {
                Text(L("↑ ↓ to navigate")); Spacer(); Text(L("esc to close"))
            }.font(.system(size: 10)).foregroundStyle(.secondary).padding(.horizontal, 14).padding(.vertical, 9)
        }
        .frame(width: 310)
        .onAppear { focusedAction = controller.items.first?.id }
        .onChange(of: controller.selectedIndex) { _, index in
            if controller.items.indices.contains(index) { focusedAction = controller.items[index].id }
        }
        .onChange(of: focusedAction) { _, id in
            if let index = controller.items.firstIndex(where: { $0.id == id }) { controller.select(index) }
        }
        .background {
            if opaque { RoundedRectangle(cornerRadius: 14).fill(Color(nsColor: .windowBackgroundColor)) }
            else { RoundedRectangle(cornerRadius: 14).fill(.regularMaterial) }
        }
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(Color.primary.opacity(colorScheme == .dark ? 0.17 : 0.1), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 14))
    }
}
