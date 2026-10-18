import AppKit
import SwiftUI

struct LocalAssistantView: View {
    @ObservedObject var service: LocalAssistantService
    var onSend: () -> Void

    var body: some View {
        VStack(spacing: 10) {
            HStack {
                Label(L("Local Assistant"), systemImage: "sparkles")
                    .font(.headline)
                Spacer()
                if service.isResponding {
                    ProgressView().controlSize(.small)
                    Button(L("Stop")) { service.cancel() }
                }
                Button(L("Send"), action: onSend)
                    .disabled(service.isResponding || service.availabilityMessage != nil)
            }
            if let error = service.history.error {
                Text(error).font(.caption).foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let unavailable = service.availabilityMessage {
                Text(unavailable).font(.callout).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 16) {
                        if service.messages.isEmpty {
                            Text(L("Ask questions, check the weather, browse the web, or ask Kebloop to run an action. Find past conversations in Chat History."))
                                .foregroundStyle(.secondary)
                        }
                        ForEach(service.messages) { message in
                            VStack(alignment: .leading, spacing: 5) {
                                HStack {
                                    Text(message.role == .user ? L("You") : L("Local Assistant"))
                                        .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                                    Spacer()
                                    if !message.text.isEmpty {
                                        Button {
                                            NSPasteboard.general.clearContents()
                                            NSPasteboard.general.setString(message.text, forType: .string)
                                        } label: { Image(systemName: "doc.on.doc") }
                                        .buttonStyle(.plain)
                                        .accessibilityLabel(L("Copy"))
                                    }
                                }
                                Text(LocalizedStringKey(message.text))
                                    .textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                        if let status = service.status, status != service.history.error {
                            Text(status).font(.caption).foregroundStyle(.secondary)
                        }
                        Color.clear.frame(height: 1).id("bottom")
                    }
                    .padding(.vertical, 8)
                }
                .onChange(of: service.messages.last?.text) { _, _ in proxy.scrollTo("bottom", anchor: .bottom) }
                .onChange(of: service.status) { _, _ in proxy.scrollTo("bottom", anchor: .bottom) }
            }
        }
        .padding(12)
    }
}
