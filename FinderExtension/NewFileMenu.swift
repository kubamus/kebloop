import AppKit

/// Finder sends menu actions across processes. Only the item's tag identifies
/// the request; representedObject is not transported back to the extension.
final class FinderNewFileMenu {
    private struct CreationRequest {
        let type: NewFileType
        let directory: URL
    }

    // Keep recent menus alive so another menu request cannot redirect an action
    // to a different folder or to a newly edited file type.
    private var snapshots: [[Int: CreationRequest]] = []
    private var nextTag = 1

    func makeMenu(configuration: NewFileConfiguration, directory: URL?, isToolbar: Bool,
                  target: AnyObject, createAction: Selector, customizeAction: Selector,
                  status: String? = nil) -> NSMenu {
        let menu = NSMenu(title: "New File")
        let actions: NSMenu
        if configuration.useSubmenu && !isToolbar {
            actions = NSMenu(title: "New File")
            let parent = NSMenuItem(title: "New File", action: nil, keyEquivalent: "")
            parent.submenu = actions
            menu.addItem(parent)
        } else { actions = menu }

        var requests: [Int: CreationRequest] = [:]
        let types = configuration.enabledTypes
        if status == nil, let directory, !types.isEmpty {
            for type in types {
                let item = NSMenuItem(title: type.title, action: createAction, keyEquivalent: "")
                item.target = target
                item.tag = nextTag
                nextTag += 1
                requests[item.tag] = CreationRequest(type: type, directory: directory)
                actions.addItem(item)
            }
        } else {
            let message = status ?? (types.isEmpty ? "Enable file types in Kebloop…" : "Choose a Finder folder to create a file")
            actions.addItem(NSMenuItem(title: message, action: nil, keyEquivalent: ""))
        }
        snapshots.append(requests)
        if snapshots.count > 8 { snapshots.removeFirst(snapshots.count - 8) }

        actions.addItem(.separator())
        let customize = NSMenuItem(title: "Customize…", action: customizeAction, keyEquivalent: "")
        customize.target = target
        actions.addItem(customize)
        return menu
    }

    func createFile(for sender: NSMenuItem) throws -> URL {
        guard let request = snapshots.reversed().compactMap({ $0[sender.tag] }).first else {
            throw StaleMenuError()
        }
        return try NewFileCreator.create(type: request.type, in: request.directory)
    }

    private struct StaleMenuError: LocalizedError {
        var errorDescription: String? { "This menu is no longer active. Open Finder’s New File menu again." }
    }
}
