import EventKit
import SwiftUI

struct CalendarMeetingSettingsView: View {
    @ObservedObject var service: CalendarMeetingService
    @State private var feedback: String?
    @State private var requesting = false

    var body: some View {
        ScrollView {
        VStack(alignment: .leading, spacing: 14) {
            Text(L("Choose which calendars Kebloop can read to find your next meeting. Calendar access is optional, and Kebloop opens a meeting link only when you choose Join."))
                .fixedSize(horizontal: false, vertical: true)

            if !service.hasReadAccess {
                Button(requesting ? L("Requesting Access…") : L("Allow Calendar Access")) {
                    requesting = true
                    Task {
                        _ = await service.requestAccess()
                        requesting = false
                    }
                }
                .disabled(requesting)
            } else {
                if service.calendars.isEmpty {
                    Text(L("No calendars are available."))
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(service.calendars) { calendar in
                        Toggle(calendar.title, isOn: Binding(
                            get: { service.selectedCalendarIdentifiers.contains(calendar.identifier) },
                            set: { service.setCalendar(calendar.identifier, selected: $0) }
                        ))
                    }
                }
                Button(L("Refresh Calendar")) { service.refresh() }
            }

            if let meeting = service.nextMeeting {
                Divider()
                VStack(alignment: .leading, spacing: 5) {
                    Text(L("Next Meeting")).font(.headline)
                    Text(meeting.title).font(.body.weight(.medium))
                    Text(meeting.relativeStart).font(.caption).foregroundStyle(.secondary)
                    if meeting.joinURL != nil {
                        Button(L("Join Next Meeting")) {
                            Task { feedback = await service.joinNextMeeting() }
                        }
                        .padding(.top, 4)
                    } else {
                        Text(L("No meeting link was found in the event URL, location, or notes."))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }

            if let message = feedback ?? service.error {
                Text(message).font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(18)
        .onAppear { service.refresh() }
        }
        .frame(minHeight: 0)
    }
}
