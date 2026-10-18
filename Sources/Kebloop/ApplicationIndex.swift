import AppKit
import Combine
import Foundation
import KebloopCore

struct IndexedApplication: Codable, Hashable, Identifiable, Sendable {
    let name: String
    let path: String
    let bundleIdentifier: String?
    let displayName: String?
    let bundleName: String?
    let executableName: String?

    init(
        name: String,
        path: String,
        bundleIdentifier: String? = nil,
        displayName: String? = nil,
        bundleName: String? = nil,
        executableName: String? = nil
    ) {
        self.name = name
        self.path = path
        self.bundleIdentifier = bundleIdentifier
        self.displayName = displayName
        self.bundleName = bundleName
        self.executableName = executableName
    }

    var id: String { path }
    var url: URL { URL(fileURLWithPath: path) }
    var searchEntry: AppSearchEntry { AppSearchEntry(id: id, name: name, path: path) }
}

@MainActor
final class ApplicationIndex: ObservableObject {
    @Published private(set) var applications: [IndexedApplication]
    @Published private(set) var isRefreshing = false
    @Published private(set) var iconRevision = 0

    var searchEnabled = true { didSet { if !searchEnabled { iconWarmupTask?.cancel() } } }
    private let iconCache = NSCache<NSString, NSImage>()
    private var pendingIconPaths = Set<String>()
    private var iconWarmupTask: Task<Void, Never>?
    private var refreshTimer: Timer?
    private var lastRefreshCompletedAt: Date?
    private let refreshInterval: TimeInterval = 5 * 60

    init() {
        iconCache.countLimit = 128
        applications = Self.loadCache()
        warmIcons(for: applications)
        refresh()

        refreshTimer = Timer.scheduledTimer(withTimeInterval: 5 * 60, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.refresh()
            }
        }
    }

    deinit {
        refreshTimer?.invalidate()
    }

    func refresh() {
        guard searchEnabled, !isRefreshing else { return }
        isRefreshing = true

        let roots = Self.applicationRoots
        let additionalApplications = Self.additionalApplications
        DispatchQueue.global(qos: .utility).async { [weak self] in
            let discovered = Self.scanApplications(roots: roots, additionalApplications: additionalApplications)
            Self.persistCache(discovered)

            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                if self.applications != discovered {
                    self.applications = discovered
                }
                self.isRefreshing = false
                self.lastRefreshCompletedAt = Date()
                if self.searchEnabled { self.warmIcons(for: discovered) }
            }
        }
    }

    func refreshIfNeeded() {
        guard searchEnabled, !isRefreshing else { return }
        guard Self.isRefreshDue(
            lastCompletedAt: lastRefreshCompletedAt,
            now: Date(),
            interval: refreshInterval
        ) else { return }
        refresh()
    }

    nonisolated static func isRefreshDue(
        lastCompletedAt: Date?,
        now: Date,
        interval: TimeInterval
    ) -> Bool {
        guard let lastCompletedAt else { return true }
        return now.timeIntervalSince(lastCompletedAt) >= interval
    }

    func icon(for app: IndexedApplication) -> NSImage? {
        let cacheKey = app.path as NSString
        if let cached = iconCache.object(forKey: cacheKey) {
            return cached
        }

        guard pendingIconPaths.insert(app.path).inserted else { return nil }

        let path = app.path
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            let loaded = self.cacheIcon(at: path)
            self.pendingIconPaths.remove(path)
            if loaded { self.iconRevision &+= 1 }
        }

        return nil
    }

    @discardableResult
    private func cacheIcon(at path: String) -> Bool {
        let key = path as NSString
        guard iconCache.object(forKey: key) == nil else { return false }

        // Preserve the workspace image's native representations. TIFF conversion
        // can fail for lazily loaded icons and discards newer system icon variants.
        // AppKit image lookup and use both stay on the main actor.
        let image = NSWorkspace.shared.icon(forFile: path)
        // Workspace icons expose many lazy representations. Summing their pixel
        // sizes can exceed a cost budget for one icon and immediately evict it.
        // Bound the number of native images without forcing representation lookup.
        iconCache.setObject(image, forKey: key)
        return true
    }

    private func warmIcons(for applications: [IndexedApplication]) {
        iconWarmupTask?.cancel()
        let paths = applications.prefix(32).map(\.path)
        iconWarmupTask = Task(priority: .utility) { @MainActor [weak self] in
            for path in paths {
                guard !Task.isCancelled, let self else { return }
                guard self.cacheIcon(at: path) else { continue }
                self.iconRevision &+= 1
                // Load one icon per turn so keyboard input gets time to run.
                do {
                    try await Task.sleep(for: .milliseconds(5))
                } catch {
                    return
                }
            }
        }
    }

    func launch(_ app: IndexedApplication, completion: @escaping (Error?) -> Void) {
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        NSWorkspace.shared.openApplication(at: app.url, configuration: configuration) { _, error in
            DispatchQueue.main.async {
                completion(error)
            }
        }
    }

    private nonisolated static var applicationRoots: [URL] {
        [
            URL(fileURLWithPath: "/Applications", isDirectory: true),
            URL(fileURLWithPath: "/System/Applications", isDirectory: true),
            URL(fileURLWithPath: "/System/Cryptexes/App/System/Applications", isDirectory: true),
            URL(fileURLWithPath: "/System/Library/CoreServices/Applications", isDirectory: true),
            FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Applications", isDirectory: true)
        ]
    }

    private nonisolated static var additionalApplications: [URL] {
        [URL(fileURLWithPath: "/System/Library/CoreServices/Finder.app", isDirectory: true)]
    }

    nonisolated static func scanApplications(roots: [URL], additionalApplications: [URL]) -> [IndexedApplication] {
        var applicationsByPath: [String: IndexedApplication] = [:]
        let keys: [URLResourceKey] = [.isDirectoryKey]

        func addApplication(at url: URL) {
            guard url.pathExtension.caseInsensitiveCompare("app") == .orderedSame,
                  (try? url.resourceValues(forKeys: Set(keys)).isDirectory) == true else { return }

            let standardizedURL = url.standardizedFileURL
            let path = standardizedURL.path
            guard applicationsByPath[path] == nil else { return }

            let bundle = Bundle(url: standardizedURL)
            if bundle?.object(forInfoDictionaryKey: "LSBackgroundOnly") as? Bool == true {
                return
            }
            let displayName = bundle?.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
            let bundleName = bundle?.object(forInfoDictionaryKey: "CFBundleName") as? String
            let bundleIdentifier = bundle?.bundleIdentifier
            let executableName = bundle?.object(forInfoDictionaryKey: "CFBundleExecutable") as? String
            let name = Self.nonempty(displayName) ?? Self.nonempty(bundleName)
                ?? standardizedURL.deletingPathExtension().lastPathComponent

            applicationsByPath[path] = IndexedApplication(
                name: name,
                path: path,
                bundleIdentifier: Self.nonempty(bundleIdentifier),
                displayName: Self.nonempty(displayName),
                bundleName: Self.nonempty(bundleName),
                executableName: Self.nonempty(executableName)
            )
        }

        for root in roots {
            guard let enumerator = FileManager.default.enumerator(
                at: root,
                includingPropertiesForKeys: keys,
                options: [.skipsHiddenFiles, .skipsPackageDescendants],
                errorHandler: { _, _ in true }
            ) else { continue }

            while let url = enumerator.nextObject() as? URL {
                guard url.pathExtension.caseInsensitiveCompare("app") == .orderedSame else { continue }
                enumerator.skipDescendants()
                addApplication(at: url)
            }
        }

        // These are explicit user-facing bundles outside the scanned roots.
        // Add them directly because a FileManager enumerator only finds children.
        for application in additionalApplications {
            addApplication(at: application)
        }

        return applicationsByPath.values.sorted {
            let order = $0.name.localizedStandardCompare($1.name)
            if order != .orderedSame { return order == .orderedAscending }
            return $0.path < $1.path
        }
    }

    private nonisolated static func loadCache() -> [IndexedApplication] {
        guard let cacheURL = cacheFileURL,
              let data = try? Data(contentsOf: cacheURL),
              let cached = try? JSONDecoder().decode([IndexedApplication].self, from: data) else {
            return []
        }

        return cached.sorted {
            let order = $0.name.localizedStandardCompare($1.name)
            if order != .orderedSame { return order == .orderedAscending }
            return $0.path < $1.path
        }
    }

    private nonisolated static func persistCache(_ applications: [IndexedApplication]) {
        guard let cacheURL = cacheFileURL,
              let data = try? JSONEncoder().encode(applications) else { return }

        do {
            try FileManager.default.createDirectory(
                at: cacheURL.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            try data.write(to: cacheURL, options: .atomic)
        } catch {
            // The live index remains useful if cache storage is unavailable.
        }
    }

    private nonisolated static var cacheFileURL: URL? {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?
            .appendingPathComponent("Kebloop", isDirectory: true)
            .appendingPathComponent("application-index.json", isDirectory: false)
    }

    private nonisolated static func nonempty(_ value: String?) -> String? {
        guard let value = value?.trimmingCharacters(in: .whitespacesAndNewlines), !value.isEmpty else {
            return nil
        }
        return value
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
