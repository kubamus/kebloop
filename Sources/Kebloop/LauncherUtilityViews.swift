import AppKit
import SwiftUI

/// The inline dictionary result shown below the launcher query field.
/// The launcher owns the query field and triggers lookup when Return is pressed.
struct LauncherDictionaryView: View {
    let word: String
    let definition: String?
    let loading: Bool
    let lookedUp: Bool
    var onLookup: () -> Void = {}

    @State private var message: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Label(word.isEmpty ? L("Dictionary") : word, systemImage: "book.closed")
                    .font(.headline)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 12)
                Button(L("Look Up"), action: onLookup)
                    .keyboardShortcut(.return, modifiers: [])
                    .disabled(word.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || loading)
            }

            Group {
                if loading {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text(L("Looking up…")).foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, minHeight: 120, alignment: .center)
                } else if let definition {
                    ScrollView {
                        Text(definition)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .textSelection(.enabled)
                            .padding(12)
                    }
                    .frame(minHeight: 112, maxHeight: .infinity)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
                    .accessibilityLabel(L("Dictionary definition"))
                } else if lookedUp {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(L("No definition was found in the active macOS dictionaries."))
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, minHeight: 112, alignment: .leading)
                } else {
                    Text(L("Look up a word in the dictionaries enabled in macOS."))
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, minHeight: 112, alignment: .leading)
                }
            }

            HStack(spacing: 10) {
                if definition != nil {
                    Button(L("Copy Definition"), action: copyDefinition)
                        .keyboardShortcut("c", modifiers: .command)
                        .disabled(definition == nil)
                }
                Button(L("Open in Dictionary"), action: openInDictionary)
                    .keyboardShortcut("o", modifiers: .command)
                    .disabled(word.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Spacer()
                if let message {
                    Text(message)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .accessibilityAddTraits(.updatesFrequently)
                } else {
                    Text(L("Return Look Up  ⌘C Copy  ⌘O Open"))
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                        .accessibilityHidden(true)
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, minHeight: 230, maxHeight: 330)
    }

    private func copyDefinition() {
        guard let definition, !definition.isEmpty else { return }

        // A selectable SwiftUI Text view is backed by NSTextView. Honor a current
        // selection first so Command-C keeps the usual macOS text-copy behavior.
        if let textView = NSApp.keyWindow?.firstResponder as? NSTextView,
           textView.selectedRange().length > 0 {
            textView.copy(nil)
            message = L("Selection copied.")
            return
        }

        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        guard pasteboard.setString(definition, forType: .string) else {
            message = L("Kebloop couldn't copy the definition.")
            return
        }
        _ = pasteboard.setPropertyList("1", forType: NSPasteboard.PasteboardType("org.nspasteboard.TransientType"))
        message = L("Definition copied.")
    }

    private func openInDictionary() {
        message = DictionaryLookupService.openInDictionary(word) ? nil : L("Kebloop couldn't open Dictionary.")
    }
}

/// Compact launcher dictation controls. Recording starts only after an explicit
/// keyboard or button action; dismissing the launcher remains the root's job.
struct LauncherDictationView: View {
    @ObservedObject var service: OnDeviceDictationService
    let application: NSRunningApplication?
    var onInsert: () -> Void = {}

    @FocusState private var focusedControl: FocusedControl?
    @State private var copyMessage: String?

    private enum FocusedControl: Hashable {
        case primaryAction
        case transcript
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Circle()
                    .fill(service.state == .recording ? Color.red : Color.secondary.opacity(0.45))
                    .frame(width: 9, height: 9)
                    .accessibilityHidden(true)
                Text(stateTitle)
                    .font(.headline)
                    .accessibilityAddTraits(service.state == .recording ? .updatesFrequently : [])
                Spacer()
                if service.state != .idle {
                    Button(L("Cancel Recording"), action: service.cancel)
                }
            }

            Text(L("On-device only. Start explicitly to grant microphone and speech access."))
                .font(.caption).foregroundStyle(.secondary)
            TextEditor(text: $service.transcript)
                .font(.system(.body, design: .rounded))
                .scrollContentBackground(.hidden)
                .padding(5)
                .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
                .focused($focusedControl, equals: .transcript)
                .accessibilityLabel(L("Dictation transcript"))
                .overlay(alignment: .topLeading) {
                    if service.transcript.isEmpty {
                        Text(L("Your transcript will appear here. You can edit it before copying or inserting."))
                            .font(.body)
                            .foregroundStyle(.tertiary)
                            .padding(.horizontal, 11)
                            .padding(.vertical, 13)
                            .allowsHitTesting(false)
                    }
                }

            if let error = service.error {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.updatesFrequently)
            }

            HStack(spacing: 8) {
                Button(primaryActionTitle, action: primaryAction)
                    .keyboardShortcut(.return, modifiers: .command)
                    .focused($focusedControl, equals: .primaryAction)
                    .disabled(service.state == .requestingPermission || service.state == .processing)

                Button(L("Edit Transcript")) { focusedControl = .transcript }
                    .keyboardShortcut("e", modifiers: .command)

                Spacer(minLength: 8)

                if !service.transcript.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Button(L("Copy"), action: copyTranscript)
                        .keyboardShortcut("c", modifiers: [.command, .shift])
                    Button(L("Insert"), action: onInsert)
                        .keyboardShortcut(.return, modifiers: [.command, .shift])
                        .disabled(service.state != .idle || application?.isTerminated == true)
                }
            }

            HStack {
                Text(L("⌘Return Start or stop  ⌘E Edit  ⇧⌘C Copy  ⇧⌘Return Insert  Esc Cancel"))
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
                Spacer(minLength: 8)
                if let copyMessage {
                    Text(copyMessage)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .accessibilityAddTraits(.updatesFrequently)
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, minHeight: 330, maxHeight: 330)
        .onAppear { focusedControl = .primaryAction }
    }

    private var primaryActionTitle: String {
        switch service.state {
        case .recording: L("Stop Dictation")
        case .idle: L("Start Dictation")
        case .requestingPermission: L("Requesting Permission…")
        case .processing: L("Finishing Transcript…")
        }
    }

    private var stateTitle: String {
        switch service.state {
        case .idle: L("Dictation")
        case .requestingPermission: L("Requesting Permissions…")
        case .recording: L("Recording on this Mac")
        case .processing: L("Finishing Transcript…")
        }
    }

    private func primaryAction() {
        switch service.state {
        case .idle:
            Task { await service.start() }
        case .recording:
            service.stop()
        case .requestingPermission, .processing:
            break
        }
    }

    private func copyTranscript() {
        copyMessage = service.copyTranscript() ? L("Transcript copied.") : L("Kebloop couldn't copy the transcript.")
    }
}
