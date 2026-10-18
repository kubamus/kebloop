import Foundation
import XCTest
@testable import Kebloop

@MainActor
final class LocalAssistantOnlineTests: XCTestCase {
    func testLiveModelGetsWeatherThroughItsWeatherTool() async throws {
        try await withModel { service in
            service.send("Check the current weather in Warsaw, Poland using your weather tool. Give the temperature in Celsius.")
            await finish(service)
            XCTAssertTrue(OnlineToolURLProtocol.requests.contains { $0.host == "geocoding-api.open-meteo.com" })
            XCTAssertTrue(OnlineToolURLProtocol.requests.contains { $0.host == "api.open-meteo.com" })
            XCTAssertTrue(service.messages.last?.text.contains("19") == true, service.messages.last?.text ?? "No answer")
        }
    }

    func testLiveModelBrowsesAPageAndFollowsItsLinks() async throws {
        try await withModel { service in
            service.send("Browse https://example.com/intro and follow its Project details link. What is the project codeword? Cite the details page.")
            await finish(service)
            XCTAssertTrue(OnlineToolURLProtocol.requests.contains { $0.path == "/intro" })
            XCTAssertTrue(OnlineToolURLProtocol.requests.contains { $0.path == "/details" })
            XCTAssertTrue(service.messages.last?.text.contains("cobalt-fern-62") == true, service.messages.last?.text ?? "No answer")
            XCTAssertTrue(service.messages.last?.text.contains("https://example.com/details") == true)
        }
    }

    private func withModel(_ body: (LocalAssistantService) async throws -> Void) async throws {
        guard ProcessInfo.processInfo.environment["KEBLOOP_TEST_LOCAL_MODEL"] == "1" else {
            throw XCTSkip("Set KEBLOOP_TEST_LOCAL_MODEL=1 to exercise the installed Apple model.")
        }
        OnlineToolURLProtocol.clear()
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [OnlineToolURLProtocol.self]
        let session = URLSession(configuration: configuration)
        let service = LocalAssistantService(tools: LocalAssistantTools(session: session), history: LocalAssistantHistoryStore(storageURL: nil))
        defer { service.cancel(); session.invalidateAndCancel(); OnlineToolURLProtocol.clear() }
        if let unavailable = service.availabilityMessage { throw XCTSkip(unavailable) }
        try await body(service)
    }

    private func finish(_ service: LocalAssistantService) async {
        for _ in 0..<240 {
            if !service.isResponding { break }
            try? await Task.sleep(for: .milliseconds(250))
        }
        XCTAssertFalse(service.isResponding, "The model timed out")
        XCTAssertNil(service.status)
    }
}

private final class OnlineToolURLProtocol: URLProtocol {
    private final class RequestLog: @unchecked Sendable {
        private let lock = NSLock()
        private var urls: [URL] = []
        func append(_ url: URL) { lock.lock(); defer { lock.unlock() }; urls.append(url) }
        func read() -> [URL] { lock.lock(); defer { lock.unlock() }; return urls }
        func clear() { lock.lock(); defer { lock.unlock() }; urls = [] }
    }
    private static let log = RequestLog()
    static var requests: [URL] { log.read() }
    static func clear() { log.clear() }
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let url = request.url else { return }
        Self.log.append(url)
        let body: String
        let contentType: String
        switch (url.host, url.path) {
        case ("geocoding-api.open-meteo.com", _):
            contentType = "application/json"
            body = #"{"results":[{"id":756135,"name":"Warsaw","latitude":52.23,"longitude":21.01,"country":"Poland","country_code":"PL","admin1":"Mazovia","timezone":"Europe/Warsaw","population":1700000}],"generationtime_ms":1}"#
        case ("api.open-meteo.com", _):
            contentType = "application/json"
            body = #"{"latitude":52.23,"longitude":21.01,"timezone":"Europe/Warsaw","timezone_abbreviation":"CEST","utc_offset_seconds":7200,"current_units":{"time":"iso8601","interval":"seconds","temperature_2m":"°C","relative_humidity_2m":"%","apparent_temperature":"°C","is_day":"","precipitation":"mm","rain":"mm","showers":"mm","snowfall":"cm","weather_code":"wmo code","cloud_cover":"%","wind_speed_10m":"km/h","wind_direction_10m":"°"},"current":{"time":"2026-10-18T14:00","interval":900,"temperature_2m":19,"relative_humidity_2m":55,"apparent_temperature":18,"is_day":1,"precipitation":0,"rain":0,"showers":0,"snowfall":0,"weather_code":2,"cloud_cover":30,"wind_speed_10m":11,"wind_direction_10m":180},"daily_units":{"time":"iso8601","weather_code":"wmo code","temperature_2m_max":"°C","temperature_2m_min":"°C","precipitation_probability_max":"%","precipitation_sum":"mm"},"daily":{"time":["2026-10-18","2026-10-19","2026-10-20"],"weather_code":[2,3,61],"temperature_2m_max":[22,20,17],"temperature_2m_min":[12,11,9],"precipitation_probability_max":[10,20,80],"precipitation_sum":[0,0,3]}}"#
        case ("example.com", "/intro"):
            contentType = "text/html"
            body = "<html><title>Project overview</title><main><p>The project codeword is on the details page.</p><a href='/details'>Project details</a></main></html>"
        case ("example.com", "/details"):
            contentType = "text/html"
            body = "<html><title>Project details</title><main><p>The project codeword is cobalt-fern-62.</p></main></html>"
        default:
            client?.urlProtocol(self, didFailWithError: URLError(.resourceUnavailable))
            return
        }
        let data = Data(body.utf8)
        let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1",
            headerFields: ["Content-Type": contentType, "Content-Length": String(data.count)])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
