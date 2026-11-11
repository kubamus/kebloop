import AppKit
import FinderSync
import OSLog

@objc(KebloopFinderSync)
final class KebloopFinderSync: FIFinderSync {
    private let menus = FinderNewFileMenu()
    private let log = Logger(subsystem: "com.kebloop.launcher.finder", category: "New File")

    override init() {
        super.init()
        FIFinderSyncController.default().directoryURLs = [URL(fileURLWithPath: "/", isDirectory: true)]
    }

    override var toolbarItemName: String { "New File" }
    override var toolbarItemToolTip: String { "Create a file with Kebloop" }
    override var toolbarItemImage: NSImage {
        NSImage(systemSymbolName: "doc.badge.plus", accessibilityDescription: "New File") ?? NSImage()
    }

    override func menu(for menuKind: FIMenuKind) -> NSMenu? {
        let controller = FIFinderSyncController.default()
        let target = controller.targetedURL() ?? controller.selectedItemURLs()?.first
        let directory = target.map { url in
            (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true
                ? url : url.deletingLastPathComponent()
        }
        do {
            let config = try NewFileConfiguration.load()
            return menus.makeMenu(configuration: config, directory: directory,
                                  isToolbar: menuKind == .toolbarItemMenu, target: self,
                                  createAction: #selector(createFile(_:)), customizeAction: #selector(customize(_:)))
        } catch {
            log.error("Could not read settings: \(error.localizedDescription)")
            return menus.makeMenu(configuration: .init(types: [], useSubmenu: true), directory: directory,
                                  isToolbar: menuKind == .toolbarItemMenu, target: self,
                                  createAction: #selector(createFile(_:)), customizeAction: #selector(customize(_:)),
                                  status: "Could not read file types — open Kebloop settings")
        }
    }

    @objc private func createFile(_ sender: NSMenuItem) {
        do {
            let url = try menus.createFile(for: sender)
            log.info("Created a file from Finder menu tag \(sender.tag)")
            NSWorkspace.shared.activateFileViewerSelecting([url])
        } catch {
            log.error("Could not create file: \(error.localizedDescription)")
            let alert = NSAlert()
            alert.messageText = "Couldn’t create the file"
            alert.informativeText = error.localizedDescription
            NSApp.activate(ignoringOtherApps: true)
            alert.runModal()
        }
    }

    @objc private func customize(_ sender: NSMenuItem) {
        guard let url = URL(string: "kebloop://new-file-settings") else { return }
        let host = Bundle.main.bundleURL.deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        NSWorkspace.shared.open([url], withApplicationAt: host, configuration: NSWorkspace.OpenConfiguration())
    }
}
