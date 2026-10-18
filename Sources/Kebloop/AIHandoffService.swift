import AppKit
import Combine
import Foundation
import KebloopCore

enum AIProvider: String, CaseIterable, Identifiable, Codable, Sendable {
    case openai
    case claude
    case local

    var id: String { rawValue }

    var title: String {
        switch self {
        case .openai: L("ChatGPT")
        case .claude: L("Claude")
        case .local: L("Local Assistant")
        }
    }

    var settingsTitle: String {
        switch self {
        case .openai: L("OpenAI (ChatGPT)")
        case .claude: L("Claude")
        case .local: L("Local Assistant")
        }
    }

    var websiteURL: URL? {
        switch self {
        case .openai: URL(string: "https://chatgpt.com/")
        case .claude: URL(string: "https://claude.ai/new")
        case .local: nil
        }
    }

    func promptURL(_ prompt: String) -> URL? {
        switch self {
        case .openai: AIPromptURL.chatGPT(prompt)
        case .claude: AIPromptURL.claude(prompt)
        case .local: nil
        }
    }

    var explanation: String {
        switch self {
        case .openai: L("Opens ChatGPT with your query already filled in.")
        case .claude: L("Opens Claude with your query in the link. You may need to sign in first.")
        case .local: L("Local Assistant chats inline in Kebloop using Apple Foundation Models on macOS 26 with Apple Intelligence. Web search and other online tools send requests over the internet.")
        }
    }
}

@MainActor
final class AIHandoffService: ObservableObject {
    static let providerStorageKey = "aiProvider"
    static let transientPasteboardType = NSPasteboard.PasteboardType("org.nspasteboard.TransientType")

    @Published var provider: AIProvider {
        didSet { defaults.set(provider.rawValue, forKey: Self.providerStorageKey) }
    }

    private let defaults: UserDefaults
    private let pasteboard: NSPasteboard
    private let openURL: (URL) -> Bool

    init(
        defaults: UserDefaults = .standard,
        pasteboard: NSPasteboard = .general,
        openURL: @escaping (URL) -> Bool = { NSWorkspace.shared.open($0) }
    ) {
        self.defaults = defaults
        self.pasteboard = pasteboard
        self.openURL = openURL

        let storedValue = defaults.string(forKey: Self.providerStorageKey)
        let storedProvider = storedValue.flatMap(AIProvider.init(rawValue:))
        provider = storedProvider ?? .openai
        if storedProvider == nil {
            defaults.set(AIProvider.openai.rawValue, forKey: Self.providerStorageKey)
        }
    }

    /// Opens a prompt-bearing cloud URL. Local Assistant is handled inline by the launcher.
    func handoff(_ text: String, completion: @escaping (String?) -> Void) {
        let prompt = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !prompt.isEmpty else {
            completion("Enter a prompt first.")
            return
        }
        guard provider != .local else {
            completion(L("Local Assistant chat is available inline in the launcher. Press Tab on a query to open it."))
            return
        }
        guard copyPrompt(prompt) else {
            completion("Kebloop couldn't copy the prompt to the clipboard.")
            return
        }

        if let websiteURL = provider.promptURL(prompt) {
            guard openURL(websiteURL) else {
                completion("Kebloop couldn't open \(provider.title) in your browser.")
                return
            }
            completion(nil)
            return
        }

        completion("Kebloop couldn't open \(provider.title) in your browser.")
    }

    private func copyPrompt(_ prompt: String) -> Bool {
        pasteboard.clearContents()
        let wrotePrompt = pasteboard.setString(prompt, forType: .string)
        let markedTransient = pasteboard.setPropertyList("1", forType: Self.transientPasteboardType)
        return wrotePrompt && markedTransient
    }
}
