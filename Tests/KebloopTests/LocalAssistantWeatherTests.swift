import Foundation
import XCTest
@testable import Kebloop

@MainActor
final class LocalAssistantWeatherTests: XCTestCase {
    func testWeatherResolvesRecognizedCityAndRequestsDeclaredUnits() async throws {
        WeatherURLProtocol.install { request in
            switch request.url?.host {
            case "geocoding-api.open-meteo.com":
                return .json(#"{"results":[{"name":"Warsaw","latitude":52.22977,"longitude":21.01178,"feature_code":"PPLC","country_code":"PL","country":"Poland","admin1":"Mazovia","population":1702139},{"name":"Warsaw","latitude":41.2381,"longitude":-85.85305,"feature_code":"PPLA2","country_code":"US","country":"United States","admin1":"Indiana","population":14472}]}"#)
            case "api.open-meteo.com":
                return .json(Self.forecast(days: 3))
            default:
                return .json("{}", statusCode: 404)
            }
        }
        let session = makeSession()
        defer { session.invalidateAndCancel(); WeatherURLProtocol.clear() }

        let output = try await LocalAssistantWeatherService(session: session).weather(location: "Warsaw", days: 3)

        XCTAssertTrue(output.contains("Weather for Warsaw, Mazovia, Poland"))
        XCTAssertTrue(output.contains("Timezone: Europe/Warsaw"))
        XCTAssertTrue(output.contains("Current observation: 2026-10-18T12:30 (Europe/Warsaw)"))
        XCTAssertTrue(output.contains("Temperature: 18.2 °C; feels like 17.6 °C"))
        XCTAssertTrue(output.contains("Condition: Partly cloudy (WMO 2)"))
        XCTAssertTrue(output.contains("Precipitation: 0.2 mm"))
        XCTAssertTrue(output.contains("Wind: 12.4 km/h, direction 245.0°"))
        XCTAssertTrue(output.contains("2026-10-18: Partly cloudy; high 20.1 °C, low 11.0 °C; precipitation chance 30.0%; total 1.2 mm"))
        XCTAssertTrue(output.contains("Open-Meteo"))
        XCTAssertTrue(output.contains("CC BY 4.0"))
        XCTAssertTrue(output.contains("Source: Open-Meteo forecast (https://api.open-meteo.com/v1/forecast?"))
        XCTAssertLessThanOrEqual(output.utf8.count, 5_000)

        let requests = WeatherURLProtocol.recordedRequests()
        let geocodingRequest = try XCTUnwrap(requests.first { $0.url?.host == "geocoding-api.open-meteo.com" })
        XCTAssertEqual(URLComponents(url: try XCTUnwrap(geocodingRequest.url), resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "name" }?.value, "Warsaw")
        XCTAssertEqual(geocodingRequest.timeoutInterval, 12)

        let forecastRequest = try XCTUnwrap(requests.first { $0.url?.host == "api.open-meteo.com" })
        let items = URLComponents(url: try XCTUnwrap(forecastRequest.url), resolvingAgainstBaseURL: false)?.queryItems ?? []
        XCTAssertEqual(items.first { $0.name == "temperature_unit" }?.value, "celsius")
        XCTAssertEqual(items.first { $0.name == "wind_speed_unit" }?.value, "kmh")
        XCTAssertEqual(items.first { $0.name == "precipitation_unit" }?.value, "mm")
        XCTAssertEqual(items.first { $0.name == "timezone" }?.value, "auto")
        XCTAssertEqual(items.first { $0.name == "forecast_days" }?.value, "3")
    }

    func testUnicodeCityAndCountryAreEncodedAsGeocodingQuery() async throws {
        WeatherURLProtocol.install { request in
            switch request.url?.host {
            case "geocoding-api.open-meteo.com":
                return .json(#"{"results":[{"name":"Łódź","latitude":51.7592,"longitude":19.4560,"country_code":"PL","country":"Poland","admin1":"Łódź Voivodeship","population":670000}]}"#)
            case "api.open-meteo.com":
                return .json(Self.forecast(days: 1))
            default:
                return .json("{}", statusCode: 404)
            }
        }
        let session = makeSession()
        defer { session.invalidateAndCancel(); WeatherURLProtocol.clear() }

        let output = try await LocalAssistantWeatherService(session: session).weather(location: "Łódź, Poland", days: 1)

        XCTAssertTrue(output.contains("Weather for Łódź, Łódź Voivodeship, Poland"))
        let request = try XCTUnwrap(WeatherURLProtocol.recordedRequests().first)
        let items = URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)?.queryItems ?? []
        XCTAssertEqual(items.first { $0.name == "name" }?.value, "Łódź, Poland")
    }

    func testAmbiguousCityReturnsCandidatesWithoutChoosingOne() async throws {
        WeatherURLProtocol.install { request in
            guard request.url?.host == "geocoding-api.open-meteo.com" else { return .json("{}", statusCode: 500) }
            return .json(#"{"results":[{"name":"Springfield","latitude":39.8,"longitude":-89.6,"country_code":"US","country":"United States","admin1":"Illinois","population":114000},{"name":"Springfield","latitude":37.2,"longitude":-93.3,"country_code":"US","country":"United States","admin1":"Missouri","population":168000}]}"#)
        }
        let session = makeSession()
        defer { session.invalidateAndCancel(); WeatherURLProtocol.clear() }

        let output = try await LocalAssistantWeatherService(session: session).weather(location: "Springfield")

        XCTAssertTrue(output.contains("Multiple locations match"))
        XCTAssertTrue(output.contains("Springfield, Illinois, United States"))
        XCTAssertTrue(output.contains("Springfield, Missouri, United States"))
        XCTAssertEqual(WeatherURLProtocol.recordedRequests().count, 1)
    }

    func testCountryCodeAndStateQualifierArePassedWithoutConfusion() async throws {
        WeatherURLProtocol.install { request in
            switch request.url?.host {
            case "geocoding-api.open-meteo.com":
                return .json(#"{"results":[{"name":"Los Angeles","latitude":34.05,"longitude":-118.24,"country_code":"US","country":"United States","admin1":"California","population":3898747}]}"#)
            case "api.open-meteo.com":
                return .json(Self.forecast(days: 3))
            default:
                return .json("{}", statusCode: 500)
            }
        }
        let session = makeSession()
        defer { session.invalidateAndCancel(); WeatherURLProtocol.clear() }
        let service = LocalAssistantWeatherService(session: session)

        _ = try await service.weather(location: "Los Angeles, CA")
        let stateItems = URLComponents(url: try XCTUnwrap(WeatherURLProtocol.recordedRequests().first?.url), resolvingAgainstBaseURL: false)?.queryItems ?? []
        XCTAssertEqual(stateItems.first { $0.name == "name" }?.value, "Los Angeles, CA")
        XCTAssertNil(stateItems.first { $0.name == "countryCode" })

        WeatherURLProtocol.clear()
        WeatherURLProtocol.install { request in
            switch request.url?.host {
            case "geocoding-api.open-meteo.com":
                return .json(#"{"results":[{"name":"Warsaw","latitude":52.2,"longitude":21.0,"country_code":"PL","country":"Poland","admin1":"Mazovia","feature_code":"PPLC","population":1700000}]}"#)
            case "api.open-meteo.com":
                return .json(Self.forecast(days: 3))
            default:
                return .json("{}", statusCode: 500)
            }
        }
        _ = try await service.weather(location: "Warsaw, PL")
        let countryItems = URLComponents(url: try XCTUnwrap(WeatherURLProtocol.recordedRequests().first?.url), resolvingAgainstBaseURL: false)?.queryItems ?? []
        XCTAssertEqual(countryItems.first { $0.name == "name" }?.value, "Warsaw")
        XCTAssertEqual(countryItems.first { $0.name == "countryCode" }?.value, "PL")
    }

    func testNoResultsAndHTTPAndForecastErrorsAreReported() async throws {
        let session = makeSession()
        defer { session.invalidateAndCancel(); WeatherURLProtocol.clear() }
        let service = LocalAssistantWeatherService(session: session)

        WeatherURLProtocol.install { _ in .json(#"{"results":[]}"#) }
        do {
            _ = try await service.weather(location: "Noplace")
            XCTFail("Expected no-results error")
        } catch let error as LocalAssistantWeatherError {
            XCTAssertTrue(error.localizedDescription.contains("No locations matched"))
        }

        WeatherURLProtocol.clear()
        WeatherURLProtocol.install { _ in .json(#"{"error":true,"reason":"rate limit"}"#, statusCode: 429) }
        do {
            _ = try await service.weather(location: "Warsaw")
            XCTFail("Expected geocoding API error")
        } catch let error as LocalAssistantWeatherError {
            XCTAssertTrue(error.localizedDescription.contains("HTTP 429"))
            XCTAssertTrue(error.localizedDescription.contains("rate limit"))
        }

        WeatherURLProtocol.clear()
        WeatherURLProtocol.install { request in
            if request.url?.host == "geocoding-api.open-meteo.com" {
                return .json(#"{"results":[{"name":"Warsaw","latitude":52.2,"longitude":21,"country":"Poland","country_code":"PL","feature_code":"PPLC","population":1700000}]}"#)
            }
            return .json(#"{"error":true,"reason":"forecast unavailable"}"#, statusCode: 503)
        }
        do {
            _ = try await service.weather(location: "Warsaw")
            XCTFail("Expected forecast API error")
        } catch let error as LocalAssistantWeatherError {
            XCTAssertTrue(error.localizedDescription.contains("HTTP 503"))
            XCTAssertTrue(error.localizedDescription.contains("forecast unavailable"))
        }
    }

    func testMalformedAndOversizedResponsesFailWithinByteLimit() async throws {
        let session = makeSession()
        defer { session.invalidateAndCancel(); WeatherURLProtocol.clear() }
        let service = LocalAssistantWeatherService(session: session)

        WeatherURLProtocol.install { _ in .json("not JSON") }
        do {
            _ = try await service.weather(location: "Warsaw")
            XCTFail("Expected a decoding error")
        } catch let error as LocalAssistantWeatherError {
            XCTAssertTrue(error.localizedDescription.contains("invalid weather data"))
        }

        WeatherURLProtocol.clear()
        WeatherURLProtocol.install { _ in .data(Data(repeating: 65, count: 128_001)) }
        do {
            _ = try await service.weather(location: "Warsaw")
            XCTFail("Expected a response size limit")
        } catch let error as LocalAssistantWeatherError {
            XCTAssertTrue(error.localizedDescription.contains("exceeded the allowed size"))
        }
    }

    func testCancelledWeatherRequestStopsBeforeAnyNetworkRequest() async throws {
        let session = makeSession()
        defer { session.invalidateAndCancel(); WeatherURLProtocol.clear() }
        let service = LocalAssistantWeatherService(session: session)
        let task = Task { @MainActor in try await service.weather(location: "Warsaw") }
        task.cancel()

        do {
            _ = try await task.value
            XCTFail("Expected cancellation")
        } catch is CancellationError {
            XCTAssertTrue(WeatherURLProtocol.recordedRequests().isEmpty)
        } catch let error as URLError {
            XCTAssertEqual(error.code, .cancelled)
        }
    }

    func testLiveWarsawWeatherWhenExplicitlyEnabled() async throws {
        guard ProcessInfo.processInfo.environment["KEBLOOP_TEST_LOCAL_MODEL"] == "1" else {
            throw XCTSkip("Set KEBLOOP_TEST_LOCAL_MODEL=1 to query Open-Meteo live.")
        }

        let output = try await LocalAssistantWeatherService().weather(location: "Warsaw, Poland")

        XCTAssertTrue(output.contains("Warsaw, Mazovia, Poland"), output)
        XCTAssertTrue(output.contains("Timezone: Europe/Warsaw"), output)
        XCTAssertTrue(output.contains("Open-Meteo"), output)
    }

    private func makeSession() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [WeatherURLProtocol.self]
        return URLSession(configuration: configuration)
    }

    private nonisolated static func forecast(days: Int) -> String {
        let dates = (0..<days).map { day in "2026-10-\(String(format: "%02d", 18 + day))" }
        let payload: [String: Any] = [
            "timezone": "Europe/Warsaw",
            "current": [
                "time": "2026-10-18T12:30",
                "temperature_2m": 18.2,
                "apparent_temperature": 17.6,
                "precipitation": 0.2,
                "weather_code": 2,
                "wind_speed_10m": 12.4,
                "wind_direction_10m": 245
            ],
            "daily": [
                "time": dates,
                "weather_code": Array(repeating: 2, count: days),
                "temperature_2m_max": Array(repeating: 20.1, count: days),
                "temperature_2m_min": Array(repeating: 11.0, count: days),
                "precipitation_probability_max": Array(repeating: 30, count: days),
                "precipitation_sum": Array(repeating: 1.2, count: days)
            ]
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload), let result = String(data: data, encoding: .utf8) else {
            return "{}"
        }
        return result
    }
}

private final class WeatherURLProtocol: URLProtocol {
    struct Reply {
        let statusCode: Int
        let body: Data

        static func json(_ string: String, statusCode: Int = 200) -> Reply {
            Reply(statusCode: statusCode, body: Data(string.utf8))
        }

        static func data(_ data: Data, statusCode: Int = 200) -> Reply {
            Reply(statusCode: statusCode, body: data)
        }
    }

    private final class State {
        let lock = NSLock()
        private var handler: ((URLRequest) -> Reply)?
        private var requests: [URLRequest] = []

        func install(_ handler: @escaping (URLRequest) -> Reply) {
            lock.lock()
            self.handler = handler
            requests = []
            lock.unlock()
        }

        func reply(for request: URLRequest) -> Reply? {
            lock.lock()
            requests.append(request)
            let handler = self.handler
            lock.unlock()
            return handler?(request)
        }

        func snapshot() -> [URLRequest] {
            lock.lock()
            defer { lock.unlock() }
            return requests
        }

        func clear() {
            lock.lock()
            handler = nil
            requests = []
            lock.unlock()
        }
    }

    private static let state = State()

    static func install(handler: @escaping (URLRequest) -> Reply) {
        state.install(handler)
    }

    static func clear() {
        state.clear()
    }

    static func recordedRequests() -> [URLRequest] {
        return state.snapshot()
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let incomingRequest = request
        guard let reply = Self.reply(for: incomingRequest),
              let url = incomingRequest.url,
              let response = HTTPURLResponse(url: url, statusCode: reply.statusCode, httpVersion: "HTTP/1.1", headerFields: ["Content-Type": "application/json"]) else {
            client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
            return
        }
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: reply.body)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() { }

    private static func reply(for request: URLRequest) -> Reply? {
        state.reply(for: request)
    }
}
