import Foundation

/// Stable identifiers shared by launcher menus and future command integrations.
enum ResultAction: String, CaseIterable {
    case editNote, copyMarkdown, exportNote, deleteNote
    case open, clearCalculationHistory, copy, pastePlainText, quitApplication, forceQuitApplication, restartApplication, hideApplication, reveal, copyPath, openWith, editQuicklink, pin, unpin, runWithInput, removeClipboardItem
    enum Kind { case application, file, quicklink, command, appleShortcut, clipboard, other }
    static func available(kind: Kind, pinned: Bool = false, running: Bool = false, hasPlainText: Bool = true) -> [ResultAction] {
        if kind == .clipboard { return [.open, .copy] + (hasPlainText ? [.pastePlainText] : []) + [pinned ? .unpin : .pin, .removeClipboardItem] }
        var actions: [ResultAction] = [.open]
        switch kind {
        case .file: actions += [.reveal, .copyPath, .openWith]
        case .application:
            if running { actions += [.hideApplication, .quitApplication, .forceQuitApplication, .restartApplication] }
            actions += [.reveal, .copyPath]
        case .quicklink: actions += [.editQuicklink]
        case .appleShortcut: actions += [.runWithInput]
        case .clipboard, .command, .other: break
        }
        if kind != .other && kind != .appleShortcut { actions.append(pinned ? .unpin : .pin) }
        return actions
    }
    var title: String {
        switch self {
        case .clearCalculationHistory: return L("Clear Calculator History…")
        case .editNote: return L("Edit Note")
        case .copyMarkdown: return L("Copy Markdown")
        case .exportNote: return L("Export Markdown…")
        case .deleteNote: return L("Delete Note")
        case .copy: return L("Copy")
        case .pastePlainText: return L("Paste as Plain Text")
        case .quitApplication: return L("Quit Application")
        case .forceQuitApplication: return L("Force Quit Application…")
        case .restartApplication: return L("Restart Application")
        case .hideApplication: return L("Hide Application")
        case .removeClipboardItem: return L("Remove Item")
        case .open: return L("Open")
        case .runWithInput: return L("Run with Input…")
        case .reveal: return L("Reveal in Finder")
        case .copyPath: return L("Copy Path")
        case .openWith: return L("Open With…")
        case .editQuicklink: return L("Edit Quicklink…")
        case .pin: return L("Pin to Favorites")
        case .unpin: return L("Unpin from Favorites")
        }
    }
}
