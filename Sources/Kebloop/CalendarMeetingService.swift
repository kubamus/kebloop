import AppKit
import Combine
import EventKit
import Foundation

struct CalendarOption: Identifiable, Equatable, Sendable {
    let identifier: String
    let title: String
    var id: String { identifier }
}

struct CalendarMeeting: Identifiable, Equatable, Sendable {
    let id: String
    let calendarIdentifier: String
    let title: String
    let startDate: Date
    let endDate: Date
    let joinURL: URL?

    var relativeStart: String { startDate.formatted(date: .abbreviated, time: .shortened) }
}

private struct CalendarSnapshot: Sendable {
    let calendars: [CalendarOption]
    let meeting: CalendarMeeting?
}

@MainActor
final class CalendarMeetingService: ObservableObject {
    static let selectedCalendarsKey = "calendarMeetingSelectedCalendars"

    @Published private(set) var calendars: [CalendarOption] = []
    @Published private(set) var selectedCalendarIdentifiers: Set<String>
    @Published private(set) var nextMeeting: CalendarMeeting?
    @Published private(set) var status: EKAuthorizationStatus
    @Published private(set) var error: String?

    private let eventStore: EKEventStore
    private let defaults: UserDefaults
    private let openURL: (URL) -> Bool
    private let now: () -> Date
    private var refreshID = UUID()

    init(
        eventStore: EKEventStore = EKEventStore(),
        defaults: UserDefaults = .standard,
        now: @escaping () -> Date = Date.init,
        openURL: @escaping (URL) -> Bool = { NSWorkspace.shared.open($0) }
    ) {
        self.eventStore = eventStore
        self.defaults = defaults
        self.now = now
        self.openURL = openURL
        selectedCalendarIdentifiers = Set(defaults.stringArray(forKey: Self.selectedCalendarsKey) ?? [])
        status = EKEventStore.authorizationStatus(for: .event)
        if Self.hasReadAccess(status) { refresh() }
    }

    var hasReadAccess: Bool { Self.hasReadAccess(status) }

    func requestAccess() async -> Bool {
        do {
            let granted = try await eventStore.requestFullAccessToEvents()
            status = EKEventStore.authorizationStatus(for: .event)
            guard granted, hasReadAccess else {
                error = L("Calendar access was not granted.")
                return false
            }
            error = nil
            await refreshFromStore()
            return true
        } catch {
            status = EKEventStore.authorizationStatus(for: .event)
            self.error = error.localizedDescription
            return false
        }
    }

    /// EventKit's synchronous enumeration runs away from the UI thread.
    func refresh() {
        status = EKEventStore.authorizationStatus(for: .event)
        guard hasReadAccess else {
            refreshID = UUID()
            calendars = []
            nextMeeting = nil
            return
        }
        let requestID = UUID()
        refreshID = requestID
        Task {
            let snapshot = await Self.makeSnapshot(selectedIdentifiers: selectedCalendarIdentifiers, now: now())
            guard refreshID == requestID else { return }
            calendars = snapshot.calendars
            nextMeeting = snapshot.meeting
        }
    }

    func setCalendar(_ identifier: String, selected: Bool) {
        if selected { selectedCalendarIdentifiers.insert(identifier) }
        else { selectedCalendarIdentifiers.remove(identifier) }
        defaults.set(selectedCalendarIdentifiers.sorted(), forKey: Self.selectedCalendarsKey)
        refresh()
    }

    /// Opens a meeting link only after the user invokes this command.
    @discardableResult
    func joinNextMeeting() async -> String? {
        guard hasReadAccess else { return L("Allow Calendar access in Kebloop settings first.") }
        let snapshot = await Self.makeSnapshot(selectedIdentifiers: selectedCalendarIdentifiers, now: now())
        calendars = snapshot.calendars
        nextMeeting = snapshot.meeting
        guard let meeting = snapshot.meeting else { return L("There are no upcoming meetings in your selected calendars.") }
        guard let url = meeting.joinURL else { return L("The next meeting does not include a join link.") }
        guard openURL(url) else { return L("Kebloop couldn't open the meeting link.") }
        return nil
    }

    private func refreshFromStore() async {
        let requestID = UUID()
        refreshID = requestID
        let snapshot = await Self.makeSnapshot(selectedIdentifiers: selectedCalendarIdentifiers, now: now())
        guard refreshID == requestID else { return }
        calendars = snapshot.calendars
        nextMeeting = snapshot.meeting
    }

    private nonisolated static func makeSnapshot(selectedIdentifiers: Set<String>, now: Date) async -> CalendarSnapshot {
        let operation = Task.detached(priority: .userInitiated) {
            let store = EKEventStore()
            let allCalendars = store.calendars(for: .event)
            let options = allCalendars
                .map { CalendarOption(identifier: $0.calendarIdentifier, title: $0.title) }
                .sorted { $0.title.localizedStandardCompare($1.title) == .orderedAscending }
            let selectedCalendars = allCalendars.filter { selectedIdentifiers.contains($0.calendarIdentifier) }
            guard !selectedCalendars.isEmpty,
                  let end = Calendar.current.date(byAdding: .day, value: 30, to: now) else {
                return CalendarSnapshot(calendars: options, meeting: nil)
            }
            let predicate = store.predicateForEvents(withStart: now, end: end, calendars: selectedCalendars)
            let event = store.events(matching: predicate)
                .filter { isEligible(isCanceled: $0.status == .canceled, allDay: $0.isAllDay, endDate: $0.endDate, currentUserDeclined: ($0.attendees ?? []).contains { $0.isCurrentUser && $0.participantStatus == .declined }, at: now) }
                .sorted { $0.startDate < $1.startDate }
                .first
            let meeting = event.map { event in
                let title = event.title?.trimmingCharacters(in: .whitespacesAndNewlines)
                return CalendarMeeting(
                    id: event.eventIdentifier ?? "\(event.calendar.calendarIdentifier):\(event.startDate.timeIntervalSince1970)",
                    calendarIdentifier: event.calendar.calendarIdentifier,
                    title: title?.isEmpty == false ? (title ?? "") : L("Untitled meeting"),
                    startDate: event.startDate,
                    endDate: event.endDate,
                    joinURL: meetingURL(eventURL: event.url, location: event.location, notes: event.notes)
                )
            }
            return CalendarSnapshot(calendars: options, meeting: meeting)
        }
        return await operation.value
    }

    nonisolated static func isEligible(isCanceled: Bool, allDay: Bool, endDate: Date, currentUserDeclined: Bool, at date: Date) -> Bool {
        !isCanceled && !allDay && endDate > date && !currentUserDeclined
    }

    nonisolated static func meetingURL(eventURL: URL?, location: String?, notes: String?) -> URL? {
        let candidates = [eventURL].compactMap { $0 } + [location, notes].compactMap { $0 }.flatMap(extractURLs)
        if let known = candidates.first(where: { isWebURL($0) && isMeetingHost($0) }) { return known }
        if let eventURL, isWebURL(eventURL) { return eventURL }
        return nil
    }

    private nonisolated static func extractURLs(_ value: String) -> [URL] {
        guard let expression = try? NSRegularExpression(pattern: #"https?://[^\s<>\[\]{}\"'`]+"#, options: [.caseInsensitive]) else { return [] }
        let range = NSRange(value.startIndex..., in: value)
        return expression.matches(in: value, range: range).compactMap { match in
            guard let swiftRange = Range(match.range, in: value) else { return nil }
            let candidate = String(value[swiftRange]).trimmingCharacters(in: CharacterSet(charactersIn: ".,;:!?"))
            guard let url = URL(string: candidate), isWebURL(url) else { return nil }
            return url
        }
    }

    private nonisolated static func isWebURL(_ url: URL) -> Bool {
        guard let scheme = url.scheme?.lowercased(), ["https", "http"].contains(scheme), url.host != nil else { return false }
        return true
    }

    private nonisolated static func isMeetingHost(_ url: URL) -> Bool {
        guard let host = url.host?.lowercased() else { return false }
        let suffixes = ["meet.google.com", "zoom.us", "zoomgov.com", "teams.microsoft.com", "teams.live.com", "webex.com", "gotomeet.me", "gotomeeting.com", "bluejeans.com", "whereby.com", "jitsi.org", "meet.jit.si"]
        return suffixes.contains { host == $0 || host.hasSuffix("." + $0) }
    }

    private nonisolated static func hasReadAccess(_ status: EKAuthorizationStatus) -> Bool {
        status == .fullAccess
    }
}
