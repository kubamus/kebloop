import AppKit
import SwiftUI

struct OnDeviceDictationView: View {
    let application: NSRunningApplication?
    @StateObject private var service: OnDeviceDictationService
    @State private var deliveryMessage: String?
    @State private var isInserting = false

    init(application: NSRunningApplication?, service: OnDeviceDictationService? = nil) {
        self.application = application
        _service = StateObject(wrappedValue: service ?? OnDeviceDictationService())
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                Circle()
                    .fill(service.state == .recording ? Color.red : Color.secondary.opacity(0.35))
                    .frame(width: 10, height: 10)
                Text(stateTitle).font(.headline)
                Spacer()
                if service.state == .recording {
                    Button(L("Stop")) { service.stop() }
                }
                if service.state != .idle {
                    Button(L("Cancel")) { service.cancel() }
                }
            }

            Text(L("Dictation uses macOS Speech recognition on this Mac. Kebloop requires on-device recognition support and will not send audio over the network. Click Start when you are ready; macOS will ask for microphone and speech permission."))
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            TextEditor(text: $service.transcript)
                .font(.system(.body, design: .rounded))
                .frame(minHeight: 180)
                .accessibilityLabel(L("Dictation transcript preview"))

            HStack {
                if service.state == .idle {
                    Button(L("Start Dictation")) { Task { await service.start() } }
                        .keyboardShortcut(.return, modifiers: .command)
                }
                Spacer()
                if !service.transcript.isEmpty {
                    Button(L("Copy Transcript")) {
                        deliveryMessage = service.copyTranscript() ? L("Transcript copied.") : L("Kebloop couldn't copy the transcript.")
                    }
                    Button(isInserting ? L("Inserting…") : L("Insert into Previous App")) { insert() }
                        .disabled(isInserting || service.state != .idle)
                }
            }

            if let error = service.error ?? deliveryMessage {
                Text(error).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(18)
        .frame(minWidth: 540, minHeight: 390)
        .onDisappear { service.cancel() }
    }

    private var stateTitle: String {
        switch service.state {
        case .idle: L("Dictation")
        case .requestingPermission: L("Requesting Permissions…")
        case .recording: L("Recording on this Mac")
        case .processing: L("Finishing Transcript…")
        }
    }

    private func insert() {
        isInserting = true
        deliveryMessage = nil
        Task {
            deliveryMessage = await service.insertTranscript(into: application)
            if deliveryMessage == nil { deliveryMessage = L("Transcript inserted.") }
            isInserting = false
        }
    }
}
