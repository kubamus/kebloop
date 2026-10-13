import AppKit
import SwiftUI

struct SelectedTextActionsView: View {
    let application: NSRunningApplication?
    @StateObject private var service: SelectedTextActionsService
    @ObservedObject private var settings: SelectedTextProviderSettings
    @State private var action: SelectedTextAction = .correctGrammar
    @State private var targetLanguage = "English"
    @State private var apiKey = ""
    @State private var showReplaceConfirmation = false
    @State private var feedback: String?
    @State private var transformTask: Task<Void, Never>?

    init(application: NSRunningApplication?, service: SelectedTextActionsService? = nil) {
        self.application = application
        let resolvedService = service ?? SelectedTextActionsService()
        _service = StateObject(wrappedValue: resolvedService)
        _settings = ObservedObject(wrappedValue: resolvedService.settings)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text(L("Selected Text")).font(.headline)
                Spacer()
                if let selection = service.selection {
                    Text(selection.applicationName).font(.caption).foregroundStyle(.secondary)
                }
            }

            if let selection = service.selection {
                GroupBox(L("Original")) {
                    ScrollView {
                        Text(selection.text).frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled)
                    }
                    .frame(minHeight: 70, maxHeight: 110)
                }

                HStack(alignment: .firstTextBaseline) {
                    Picker(L("Action"), selection: $action) {
                        ForEach(SelectedTextAction.allCases) { item in Text(item.title).tag(item) }
                    }
                    .frame(width: 190)
                    if action == .translate {
                        TextField(L("Target language"), text: $targetLanguage).frame(width: 150)
                    }
                    Spacer()
                    Button(service.isWorking ? L("Cancel") : action.title) {
                        if service.isWorking { cancelTransform() } else { runAction() }
                    }
                        .keyboardShortcut(.return, modifiers: .command)
                }

                HStack(alignment: .firstTextBaseline) {
                    Picker(L("Provider"), selection: $settings.provider) {
                        ForEach(SelectedTextProvider.allCases) { provider in Text(provider.title).tag(provider) }
                    }
                    .frame(width: 220)
                    switch settings.provider {
                    case .local:
                        TextField(L("Ollama model"), text: $settings.localModel).frame(width: 170)
                    case .openAI:
                        TextField(L("Model"), text: $settings.openAIModel).frame(width: 150)
                        providerKeyField
                    case .claude:
                        TextField(L("Model"), text: $settings.claudeModel).frame(width: 190)
                        providerKeyField
                    }
                }
                Text(providerPrivacyText)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                GroupBox(L("Preview")) {
                    ScrollView {
                        Text(service.preview.isEmpty ? L("Choose an action to create a preview. Nothing is replaced until you review it and choose Replace Selection.") : service.preview)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .textSelection(.enabled)
                    }
                    .frame(minHeight: 90, maxHeight: 150)
                }

                HStack {
                    if !service.preview.isEmpty {
                        Button(L("Copy Result")) { feedback = service.copyPreview() ? L("Result copied.") : L("Kebloop couldn't copy the result.") }
                            .disabled(service.isWorking)
                        Button(L("Replace Selection…")) { showReplaceConfirmation = true }
                            .keyboardShortcut(.return, modifiers: [.command, .shift])
                            .disabled(service.isWorking)
                    }
                    Spacer()
                    Button(L("Read Selection Again")) {
                        transformTask?.cancel()
                        service.cancelTransform()
                        service.captureSelection(from: application)
                    }
                    .disabled(service.isWorking)
                }
            } else {
                Text(service.error ?? L("Kebloop couldn't read selected text."))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Button(L("Try Again")) { service.captureSelection(from: application) }
            }

            if let error = service.error ?? feedback {
                Text(error).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(18)
        .frame(minWidth: 720, minHeight: 520)
        .onAppear {
            service.captureSelection(from: application)
            apiKey = settings.apiKey(for: settings.provider)
        }
        .onChange(of: settings.provider) { _, provider in apiKey = settings.apiKey(for: provider) }
        .onDisappear { cancelTransform() }
        .confirmationDialog(L("Replace the selected text in the previous app?"), isPresented: $showReplaceConfirmation, titleVisibility: .visible) {
            Button(L("Replace Selection"), role: .destructive) { replaceSelection() }
            Button(L("Cancel"), role: .cancel) { }
        } message: {
            Text(L("Kebloop rechecks the application and selected text before pasting. If replacement is unavailable, the result stays copied."))
        }
    }

    @ViewBuilder private var providerKeyField: some View {
        SecureField(L("API key"), text: $apiKey)
            .frame(minWidth: 150, maxWidth: 220)
            .onSubmit(saveAPIKey)
        Button(L("Save Key"), action: saveAPIKey)
    }

    private var providerPrivacyText: String {
        switch service.settings.provider {
        case .local: L("Selected text is sent to the Ollama service on this Mac at 127.0.0.1. Install and start Ollama before using this option.")
        case .openAI: L("Selected text is sent to OpenAI for processing. The API key is stored in the macOS Keychain.")
        case .claude: L("Selected text is sent to Anthropic for processing. The API key is stored in the macOS Keychain.")
        }
    }

    private func saveAPIKey() {
        do {
                    try settings.setAPIKey(apiKey, for: settings.provider)
            feedback = L("API key saved in the macOS Keychain.")
        } catch { feedback = error.localizedDescription }
    }

    private func runAction() {
        feedback = nil
        transformTask = Task { await service.transform(action, targetLanguage: targetLanguage) }
    }

    private func cancelTransform() {
        transformTask?.cancel()
        transformTask = nil
        service.cancelTransform()
    }

    private func replaceSelection() {
        feedback = nil
        Task {
            let failure = await service.replaceSelection(into: application)
            feedback = failure?.localizedDescription ?? L("Selected text replaced.")
        }
    }
}
