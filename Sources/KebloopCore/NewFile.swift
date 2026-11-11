import Foundation
import Darwin

public struct NewFileType: Codable, Identifiable, Equatable, Sendable {
    public var id: String
    public var fileExtension: String
    public var baseName: String
    public var menuTitle: String
    public var template: String
    public var enabled: Bool

    public init(id: String = UUID().uuidString, fileExtension: String, baseName: String,
                menuTitle: String, template: String = "", enabled: Bool = false) {
        self.id = id
        self.fileExtension = fileExtension
        self.baseName = baseName
        self.menuTitle = menuTitle
        self.template = template
        self.enabled = enabled
    }

    public var title: String {
        menuTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "New .\(fileExtension)" : menuTitle
    }

    public func validated() throws -> NewFileType {
        var copy = self
        copy.fileExtension = fileExtension.trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: ".")).lowercased()
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyz0123456789._-")
        guard !copy.fileExtension.isEmpty, copy.fileExtension.count <= 32,
              copy.fileExtension.unicodeScalars.allSatisfy({ allowed.contains($0) }) else {
            throw NewFileError.invalidExtension
        }
        guard !baseName.contains("/"), !baseName.contains(":"), !baseName.contains("\0"),
              !baseName.contains("\n"), !baseName.contains("\r") else {
            throw NewFileError.invalidName
        }
        return copy
    }

    public func filename(number: Int = 1) -> String {
        var base = baseName
        let suffix = "." + fileExtension
        if base.lowercased().hasSuffix(suffix.lowercased()) { base.removeLast(suffix.count) }
        if base.isEmpty { return suffix + (number == 1 ? "" : " \(number)") }
        return base + (number == 1 ? "" : " \(number)") + suffix
    }
}

public struct NewFileConfiguration: Codable, Equatable, Sendable {
    public var types: [NewFileType]
    public var useSubmenu: Bool

    public init(types: [NewFileType] = Self.presets, useSubmenu: Bool = false) {
        self.types = types
        self.useSubmenu = useSubmenu
    }

    public var enabledTypes: [NewFileType] { types.filter(\.enabled) }

    public static let presets: [NewFileType] = [
        .init(id: "txt", fileExtension: "txt", baseName: "New Text File", menuTitle: "New Text File", enabled: true),
        .init(id: "md", fileExtension: "md", baseName: "Untitled", menuTitle: "New Markdown"),
        .init(id: "json", fileExtension: "json", baseName: "data", menuTitle: "New JSON"),
        .init(id: "sh", fileExtension: "sh", baseName: "script", menuTitle: "New Shell Script"),
        .init(id: "env", fileExtension: "env", baseName: "", menuTitle: "New .env"),
        .init(id: "yml", fileExtension: "yml", baseName: "config", menuTitle: "New YAML"),
        .init(id: "gitignore", fileExtension: "gitignore", baseName: "", menuTitle: "New .gitignore"),
        .init(id: "html", fileExtension: "html", baseName: "index", menuTitle: "New HTML"),
        .init(id: "rtf", fileExtension: "rtf", baseName: "Untitled", menuTitle: "New Rich Text", template: "{\\rtf1}")
    ]

    // Foundation redirects even NSHomeDirectoryForUser into an app's sandbox.
    // Resolve the account record directly so the host and extension read the
    // same file. Use the reentrant API because both processes use Foundation on
    // other threads, which may also look up account records.
    public static var storageURL: URL {
        get throws {
            var account = passwd()
            var resolved: UnsafeMutablePointer<passwd>?
            var buffer = [CChar](repeating: 0, count: 16_384)
            let home = try buffer.withUnsafeMutableBufferPointer { storage -> String in
                guard getpwuid_r(getuid(), &account, storage.baseAddress, storage.count, &resolved) == 0,
                      resolved != nil, let directory = account.pw_dir, directory.pointee != 0 else {
                    throw NewFileError.homeDirectoryUnavailable
                }
                return String(cString: directory)
            }
            return URL(fileURLWithPath: home, isDirectory: true)
                .appendingPathComponent("Library/Application Support/Kebloop/new-file.json")
        }
    }

    public static func load(from url: URL? = nil) throws -> Self {
        let url = try url ?? storageURL
        let data: Data
        do { data = try Data(contentsOf: url) }
        catch let error as CocoaError where error.code == .fileReadNoSuchFile { return Self() }
        var config = try JSONDecoder().decode(Self.self, from: data)
        config.types = try config.types.map { try $0.validated() }
        guard Set(config.types.map(\.id)).count == config.types.count else { throw NewFileError.invalidConfiguration }
        return config
    }

    public func save(to url: URL? = nil) throws {
        let url = try url ?? Self.storageURL
        var config = self
        config.types = try types.map { try $0.validated() }
        guard Set(config.types.map(\.id)).count == config.types.count else { throw NewFileError.invalidConfiguration }
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try JSONEncoder().encode(config).write(to: url, options: .atomic)
    }
}

public enum NewFileError: LocalizedError {
    case invalidExtension, invalidName, invalidConfiguration, notDirectory, tooManyFiles, homeDirectoryUnavailable
    public var errorDescription: String? {
        switch self {
        case .invalidExtension: "Enter a file extension using letters, numbers, dots, hyphens, or underscores (up to 32 characters)."
        case .invalidName: "The filename cannot contain slashes, colons, or line breaks."
        case .invalidConfiguration: "The New File configuration contains duplicate identifiers."
        case .notDirectory: "Choose an existing folder for the new file."
        case .tooManyFiles: "Could not find an unused filename."
        case .homeDirectoryUnavailable: "Could not find your home folder to load New File settings."
        }
    }
}

public enum NewFileCreator {
    /// Exclusive creation also handles races with Finder or another Kebloop process.
    public static func create(type: NewFileType, in directory: URL) throws -> URL {
        let type = try type.validated()
        guard directory.isFileURL else { throw NewFileError.notDirectory }
        let scoped = directory.startAccessingSecurityScopedResource()
        defer { if scoped { directory.stopAccessingSecurityScopedResource() } }
        guard try directory.resourceValues(forKeys: [.isDirectoryKey]).isDirectory == true else {
            throw NewFileError.notDirectory
        }
        for number in 1...100_000 {
            let url = directory.appendingPathComponent(type.filename(number: number), isDirectory: false)
            do {
                try Data(type.template.utf8).write(to: url, options: .withoutOverwriting)
                return url
            } catch let error as CocoaError where error.code == .fileWriteFileExists {
                continue
            }
        }
        throw NewFileError.tooManyFiles
    }
}
