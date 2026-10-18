import Foundation

/// Searchable app identity supplied to the model without reading bundles during a request.
struct AssistantApplicationContext: Encodable {
    let id: String
    let name: String
    let path: String
    let bundleIdentifier: String?
    let displayName: String?
    let bundleName: String?
    let executableName: String?
    let aliases: [String]

    init(app: IndexedApplication, customAlias: String?) {
        id = "app:" + app.id
        name = app.name
        path = app.path
        bundleIdentifier = app.bundleIdentifier
        displayName = app.displayName
        bundleName = app.bundleName
        executableName = app.executableName
        aliases = AssistantApplicationIdentity(app: app, customAlias: customAlias).aliases
    }
}
