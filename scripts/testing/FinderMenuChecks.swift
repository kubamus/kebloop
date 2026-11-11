import AppKit

/// Exercises Finder's cross-process action contract: the returned menu item
/// has a selector and numeric tag, but no Swift representedObject.
enum FinderMenuChecks {
    private final class ActionTarget: NSObject {
        let menus = FinderNewFileMenu()
        var created: [URL] = []
        var error: Error?

        @objc func create(_ sender: NSMenuItem) {
            do { created.append(try menus.createFile(for: sender)) }
            catch { self.error = error }
        }

        @objc func customize(_ sender: NSMenuItem) {}

        func menu(_ config: NewFileConfiguration, in directory: URL?, toolbar: Bool = false) -> NSMenu {
            menus.makeMenu(configuration: config, directory: directory, isToolbar: toolbar,
                           target: self, createAction: #selector(create(_:)), customizeAction: #selector(customize(_:)))
        }

        func clickReturnedItem(_ original: NSMenuItem) throws {
            let returned = NSMenuItem(title: original.title, action: original.action, keyEquivalent: "")
            returned.tag = original.tag
            error = nil
            guard let action = returned.action, responds(to: action) else {
                throw failure("Menu action is not available on the receiver.")
            }
            _ = perform(action, with: returned)
            if let error { throw error }
        }
    }

    private static func failure(_ message: String) -> NSError {
        NSError(domain: "FinderMenuChecks", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
    }

    private static func require(_ condition: @autoclosure () -> Bool, _ message: String) throws {
        guard condition() else { throw failure(message) }
    }

    static func run(in root: URL) throws {
        let firstFolder = root.appendingPathComponent("first")
        let secondFolder = root.appendingPathComponent("second")
        try FileManager.default.createDirectory(at: firstFolder, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: secondFolder, withIntermediateDirectories: true)
        let target = ActionTarget()
        var config = NewFileConfiguration(types: [
            .init(fileExtension: "txt", baseName: "Test File", menuTitle: "New Text File", template: "original template", enabled: true),
            .init(fileExtension: "json", baseName: "data", menuTitle: "New JSON", template: "{\n}\n", enabled: true),
            .init(fileExtension: "env", baseName: "", menuTitle: "New .env", template: "KEY=value\n", enabled: true)
        ], useSubmenu: true)
        let grouped = target.menu(config, in: firstFolder)
        try require(grouped.items.count == 1, "Grouped menus must have only the New File parent at the top level.")
        guard let submenu = grouped.items.first?.submenu else { throw failure("Missing New File submenu.") }
        try require(submenu.items.last?.title == "Customize…", "Customization must be inside New File.")
        try require(submenu.items[3].isSeparatorItem, "Separate creation actions from customization inside the submenu.")

        // Opening a second menu with different settings/folder must not change
        // the meaning of an action from the first menu.
        config.types[0].template = "changed template"
        _ = target.menu(config, in: secondFolder)
        try target.clickReturnedItem(submenu.items[0])
        let first = firstFolder.appendingPathComponent("Test File.txt")
        try require(target.created.last == first, "Action must create a file in the original target folder.")
        let contents = try String(contentsOf: first, encoding: .utf8)
        try require(contents == "original template", "Action must use the original template snapshot.")
        try target.clickReturnedItem(submenu.items[0])
        try require(target.created.last?.lastPathComponent == "Test File 2.txt", "Repeated actions must increment the filename.")
        try target.clickReturnedItem(submenu.items[1])
        let json = try String(contentsOf: firstFolder.appendingPathComponent("data.json"), encoding: .utf8)
        try require(json == "{\n}\n", "JSON template must be written to disk.")
        try target.clickReturnedItem(submenu.items[2])
        try require(FileManager.default.fileExists(atPath: firstFolder.appendingPathComponent(".env").path), "Dotfile action must write a file.")

        let toolbar = target.menu(config, in: firstFolder, toolbar: true)
        try require(toolbar.items.first?.submenu == nil && toolbar.items.last?.title == "Customize…",
                    "Toolbar actions and customization must share one dropdown.")
        config.useSubmenu = false
        let flat = target.menu(config, in: firstFolder)
        try require(flat.items.first?.submenu == nil && flat.items.last?.title == "Customize…",
                    "Flat mode must retain creation and customization actions.")
        config = NewFileConfiguration(types: [], useSubmenu: true)
        let disabled = target.menu(config, in: firstFolder)
        try require(disabled.items.count == 1 && disabled.items.first?.submenu?.items.last?.title == "Customize…",
                    "Customization must stay grouped when all types are disabled.")
        do {
            _ = try target.menus.createFile(for: NSMenuItem(title: "Stale", action: nil, keyEquivalent: ""))
            throw failure("Missing menu tags must report an error.")
        } catch is CocoaError { throw failure("Unexpected filesystem error for a missing tag.") }
        catch {
            // Do not accidentally swallow the assertion itself.
            if (error as NSError).domain == "FinderMenuChecks" { throw error }
        }
        print("Finder menu grouping and tag-based file creation passed in a signed sandbox.")
    }
}
