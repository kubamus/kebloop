import AppKit
import AVFoundation
import Carbon
import Combine
import Foundation
import Speech

enum DictationState: Equatable {
    case idle
    case requestingPermission
    case recording
    case processing
}

@MainActor
final class OnDeviceDictationService: ObservableObject {
    @Published private(set) var state: DictationState = .idle
    @Published var transcript = ""
    @Published private(set) var error: String?

    private let audioEngine = AVAudioEngine()
    private let pasteboard: NSPasteboard
    private let session: SFSpeechRecognizer?
    private let onDeviceSupportOverride: Bool?
    private let permissionRequestObserver: (() -> Void)?
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var recognitionTask: SFSpeechRecognitionTask?
    private var captureID = UUID()
    private var tapInstalled = false
    private var durationTask: Task<Void, Never>?
    private var finalizationTask: Task<Void, Never>?

    init(
        locale: Locale = .current,
        pasteboard: NSPasteboard = .general,
        onDeviceSupportOverride: Bool? = nil,
        permissionRequestObserver: (() -> Void)? = nil
    ) {
        session = SFSpeechRecognizer(locale: locale)
        self.pasteboard = pasteboard
        self.onDeviceSupportOverride = onDeviceSupportOverride
        self.permissionRequestObserver = permissionRequestObserver
    }

    var supportsOnDeviceRecognition: Bool {
        onDeviceSupportOverride ?? (session?.supportsOnDeviceRecognition == true)
    }

    func start() async {
        guard state == .idle else { return }
        captureID = UUID()
        let activeCapture = captureID
        state = .requestingPermission
        transcript = ""
        error = nil

        guard supportsOnDeviceRecognition else {
            fail(L("On-device recognition is unavailable for the current language. Kebloop does not send dictation audio over the network."))
            return
        }
        guard let session else {
            fail(L("Speech recognition is unavailable for the current language."))
            return
        }
        guard session.isAvailable else {
            fail(L("Speech recognition is not available right now."))
            return
        }

        permissionRequestObserver?()
        let speechStatus = await speechAuthorization()
        guard captureID == activeCapture else { return }
        guard speechStatus == .authorized else {
            fail(L("Allow Speech Recognition in System Settings to use dictation."))
            return
        }
        let microphoneStatus = await microphoneAuthorization()
        guard captureID == activeCapture else { return }
        guard microphoneStatus else {
            fail(L("Allow microphone access in System Settings to use dictation."))
            return
        }
        let audioRequest = SFSpeechAudioBufferRecognitionRequest()
        audioRequest.shouldReportPartialResults = true
        audioRequest.requiresOnDeviceRecognition = true
        let input = audioEngine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            fail(L("Kebloop couldn't access an audio input."))
            return
        }
        input.installTap(onBus: 0, bufferSize: 1_024, format: format) { buffer, _ in audioRequest.append(buffer) }
        tapInstalled = true
        request = audioRequest
        recognitionTask = session.recognitionTask(with: audioRequest) { [weak self] result, error in
            Task { @MainActor in
                guard let self, self.captureID == activeCapture else { return }
                if let result { self.transcript = result.bestTranscription.formattedString }
                if let error {
                    self.finishAudio()
                    self.error = error.localizedDescription
                    self.state = .idle
                    return
                }
                if result?.isFinal == true {
                    self.finishAudio()
                    self.state = .idle
                }
            }
        }
        audioEngine.prepare()
        do {
            try audioEngine.start()
            state = .recording
            durationTask = Task { [weak self] in
                try? await Task.sleep(nanoseconds: 120_000_000_000)
                guard let self, self.captureID == activeCapture, self.state == .recording else { return }
                self.error = L("Dictation stopped after two minutes.")
                self.stop()
            }
        } catch {
            finishAudio()
            fail(error.localizedDescription)
        }
    }

    func stop() {
        guard state == .recording else { return }
        state = .processing
        audioEngine.stop()
        removeInputTap()
        request?.endAudio()
        let activeCapture = captureID
        finalizationTask?.cancel()
        finalizationTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 8_000_000_000)
            guard let self, self.captureID == activeCapture, self.state == .processing else { return }
            self.recognitionTask?.cancel()
            self.finishAudio()
            self.state = .idle
            if self.transcript.isEmpty { self.error = L("Dictation did not finish. Try again.") }
        }
    }

    func cancel() {
        captureID = UUID()
        durationTask?.cancel()
        durationTask = nil
        finalizationTask?.cancel()
        finalizationTask = nil
        audioEngine.stop()
        removeInputTap()
        request?.endAudio()
        recognitionTask?.cancel()
        request = nil
        recognitionTask = nil
        state = .idle
    }

    @discardableResult
    func copyTranscript() -> Bool {
        let text = transcript.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return false }
        pasteboard.clearContents()
        let copied = pasteboard.setString(text, forType: .string)
        if copied { _ = pasteboard.setPropertyList("1", forType: NSPasteboard.PasteboardType("org.nspasteboard.TransientType")) }
        return copied
    }

    /// A user-triggered insert into the app that was active before the dictation window opened.
    func insertTranscript(into application: NSRunningApplication?) async -> String? {
        guard copyTranscript() else { return L("There is no transcript to insert.") }
        guard let application, !application.isTerminated,
              application.processIdentifier != ProcessInfo.processInfo.processIdentifier else {
            return L("Transcript copied. The previous application is no longer available.")
        }
        guard AXIsProcessTrusted() else { return L("Transcript copied. Allow Kebloop in System Settings → Privacy & Security → Accessibility to insert text.") }
        guard !IsSecureEventInputEnabled() else { return L("Transcript copied. Insertion is unavailable while secure input is active.") }
        guard let secure = Self.isSecureFocusedField(processIdentifier: application.processIdentifier) else {
            return L("Transcript copied. Kebloop couldn't confirm that the previous app has a safe text field.")
        }
        guard !secure else {
            return L("Transcript copied. Dictation is disabled in secure text fields.")
        }

        let revision = pasteboard.changeCount
        guard application.activate(options: []) else { return L("Transcript copied. Kebloop couldn't activate the previous application.") }
        try? await Task.sleep(nanoseconds: 170_000_000)
        guard !Task.isCancelled else { return L("Insertion was cancelled. Transcript remains copied.") }
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == application.processIdentifier else {
            return L("Transcript copied. The previous application is no longer frontmost.")
        }
        guard !IsSecureEventInputEnabled(), let secure = Self.isSecureFocusedField(processIdentifier: application.processIdentifier), !secure else {
            return L("Transcript copied. Insertion is unavailable in secure input.")
        }
        guard pasteboard.changeCount == revision else { return L("The clipboard changed, so insertion was cancelled to preserve its new contents.") }
        guard Self.postPaste() else { return L("Transcript copied. Kebloop couldn't create the paste event.") }
        return nil
    }

    private func speechAuthorization() async -> SFSpeechRecognizerAuthorizationStatus {
        let status = SFSpeechRecognizer.authorizationStatus()
        guard status == .notDetermined else { return status }
        return await withCheckedContinuation { continuation in
            SFSpeechRecognizer.requestAuthorization { continuation.resume(returning: $0) }
        }
    }

    private func microphoneAuthorization() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return true
        case .denied, .restricted: return false
        case .notDetermined:
            return await withCheckedContinuation { continuation in
                AVCaptureDevice.requestAccess(for: .audio) { continuation.resume(returning: $0) }
            }
        @unknown default: return false
        }
    }

    private func finishAudio() {
        durationTask?.cancel()
        durationTask = nil
        finalizationTask?.cancel()
        finalizationTask = nil
        if audioEngine.isRunning { audioEngine.stop() }
        removeInputTap()
        request?.endAudio()
        request = nil
        recognitionTask = nil
    }

    private func fail(_ message: String) {
        captureID = UUID()
        recognitionTask?.cancel()
        finishAudio()
        state = .idle
        error = message
    }

    private func removeInputTap() {
        guard tapInstalled else { return }
        audioEngine.inputNode.removeTap(onBus: 0)
        tapInstalled = false
    }

    private static func isSecureFocusedField(processIdentifier: pid_t) -> Bool? {
        let application = AXUIElementCreateApplication(processIdentifier)
        AXUIElementSetMessagingTimeout(application, 0.25)
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(application, kAXFocusedUIElementAttribute as CFString, &value) == .success,
              let value, CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
        let element = unsafeBitCast(value, to: AXUIElement.self)
        var subroleValue: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXSubroleAttribute as CFString, &subroleValue) == .success,
              let subrole = subroleValue as? String else { return nil }
        let lowered = subrole.lowercased()
        return lowered.contains("secure") || lowered.contains("password")
    }

    private static func postPaste() -> Bool {
        guard let source = CGEventSource(stateID: .privateState),
              let down = CGEvent(keyboardEventSource: source, virtualKey: 9, keyDown: true),
              let up = CGEvent(keyboardEventSource: source, virtualKey: 9, keyDown: false) else { return false }
        down.flags = .maskCommand
        up.flags = .maskCommand
        down.setIntegerValueField(.eventSourceUserData, value: 0x4B45424C)
        up.setIntegerValueField(.eventSourceUserData, value: 0x4B45424C)
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
        return true
    }
}
