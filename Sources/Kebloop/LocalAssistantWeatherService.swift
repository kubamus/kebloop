import Foundation

@MainActor
final class LocalAssistantWeatherService {
    private let session: URLSession

    init(session: URLSession = .shared) {
        self.session = session
    }

    func weather(location: String, days: Int = 3) async throws -> String {
        try Task.checkCancellation()
        let query = try Self.geocodingQuery(for: location)
        let geocodingResponse = try await requestJSON(
            host: "geocoding-api.open-meteo.com",
            path: "/v1/search",
            queryItems: [
                URLQueryItem(name: "name", value: query.name),
                URLQueryItem(name: "count", value: "10"),
                URLQueryItem(name: "language", value: "en"),
                URLQueryItem(name: "format", value: "json")
            ] + (query.countryCode.map { [URLQueryItem(name: "countryCode", value: $0)] } ?? []),
            maximumBytes: 128_000
        )
        let geocoding = try decode(GeocodingResponse.self, from: geocodingResponse.data)
        let places = geocoding.results ?? []
        guard let place = places.first else {
            throw LocalAssistantWeatherError.noLocations(location.trimmingCharacters(in: .whitespacesAndNewlines))
        }

        if Self.isAmbiguous(places, requestedName: query.city) {
            let candidates = places.prefix(5).map(Self.locationLabel)
            return "Multiple locations match \(query.city.debugDescription): \(candidates.joined(separator: "; ")). Please specify a country or state."
        }

        let requestedDays = min(max(days, 1), 7)
        let forecastResponse = try await requestJSON(
            host: "api.open-meteo.com",
            path: "/v1/forecast",
            queryItems: [
                URLQueryItem(name: "latitude", value: Self.number(place.latitude)),
                URLQueryItem(name: "longitude", value: Self.number(place.longitude)),
                URLQueryItem(name: "current", value: "temperature_2m,apparent_temperature,precipitation,weather_code,wind_speed_10m,wind_direction_10m"),
                URLQueryItem(name: "daily", value: "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,precipitation_sum"),
                URLQueryItem(name: "temperature_unit", value: "celsius"),
                URLQueryItem(name: "wind_speed_unit", value: "kmh"),
                URLQueryItem(name: "precipitation_unit", value: "mm"),
                URLQueryItem(name: "timezone", value: "auto"),
                URLQueryItem(name: "forecast_days", value: String(requestedDays))
            ],
            maximumBytes: 256_000
        )
        let forecast = try decode(ForecastResponse.self, from: forecastResponse.data)
        guard let current = forecast.current else {
            throw LocalAssistantWeatherError.malformedResponse
        }

        return Self.format(
            place: place,
            current: current,
            timezone: forecast.timezone,
            daily: forecast.daily,
            days: requestedDays,
            forecastURL: forecastResponse.url
        )
    }

    private func requestJSON(
        host: String,
        path: String,
        queryItems: [URLQueryItem],
        maximumBytes: Int
    ) async throws -> (data: Data, url: URL) {
        var components = URLComponents()
        components.scheme = "https"
        components.host = host
        components.path = path
        components.queryItems = queryItems
        guard let url = components.url else { throw LocalAssistantWeatherError.invalidRequest }

        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 12)
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        let (bytes, response) = try await session.bytes(for: request)
        guard let httpResponse = response as? HTTPURLResponse,
              response.url?.scheme?.lowercased() == "https",
              response.url?.host?.lowercased() == host else {
            throw LocalAssistantWeatherError.invalidResponse
        }
        if response.expectedContentLength > Int64(maximumBytes) {
            throw LocalAssistantWeatherError.responseTooLarge
        }

        var data = Data()
        data.reserveCapacity(min(maximumBytes, 16_384))
        do {
            for try await byte in bytes {
                try Task.checkCancellation()
                guard data.count < maximumBytes else { throw LocalAssistantWeatherError.responseTooLarge }
                data.append(byte)
            }
        } catch is CancellationError {
            throw CancellationError()
        }

        guard (200..<300).contains(httpResponse.statusCode) else {
            let failure = try? JSONDecoder().decode(APIErrorResponse.self, from: data)
            throw LocalAssistantWeatherError.apiFailure(
                statusCode: httpResponse.statusCode,
                reason: failure?.reason
            )
        }
        return (data, url)
    }

    private func decode<Response: Decodable>(_ type: Response.Type, from data: Data) throws -> Response {
        do {
            if let failure = try? JSONDecoder().decode(APIErrorResponse.self, from: data), failure.error == true {
                throw LocalAssistantWeatherError.apiFailure(statusCode: nil, reason: failure.reason)
            }
            return try JSONDecoder().decode(type, from: data)
        } catch let error as LocalAssistantWeatherError {
            throw error
        } catch {
            throw LocalAssistantWeatherError.malformedResponse
        }
    }

    private static func geocodingQuery(for input: String) throws -> GeocodingQuery {
        let pieces = input.split(separator: ",", omittingEmptySubsequences: false)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        guard let city = pieces.first, !city.isEmpty else { throw LocalAssistantWeatherError.invalidLocation }
        guard city.count >= 2 else { throw LocalAssistantWeatherError.invalidLocation }

        guard pieces.count > 1, !pieces[1].isEmpty else {
            return GeocodingQuery(city: city, name: city, countryCode: nil)
        }

        let qualifier = pieces[1]
        let countryCode = pieces.count > 2 && Self.isCountryCode(pieces.last ?? "")
            ? pieces.last
            : nil
        if let countryCode {
            return GeocodingQuery(city: city, name: "\(city), \(qualifier)", countryCode: countryCode.uppercased())
        }
        if Self.isCountryCode(qualifier), !Self.usStateAbbreviations.contains(qualifier.uppercased()) {
            return GeocodingQuery(city: city, name: city, countryCode: qualifier.uppercased())
        }
        return GeocodingQuery(city: city, name: "\(city), \(qualifier)", countryCode: nil)
    }

    private static func isCountryCode(_ value: String) -> Bool {
        value.count == 2 && value.unicodeScalars.allSatisfy { CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz").contains($0) }
    }

    private static func isAmbiguous(_ places: [GeocodedPlace], requestedName: String) -> Bool {
        guard places.count > 1 else { return false }
        let exactMatches = places.filter { normalized($0.name) == normalized(requestedName) }
        if exactMatches.count == 1, places.first.map({ normalized($0.name) == normalized(requestedName) }) == true {
            return false
        }
        guard exactMatches.count > 1, let first = exactMatches.first, let second = exactMatches.dropFirst().first else { return true }
        if first.featureCode == "PPLC" { return false }
        guard let firstPopulation = first.population, let secondPopulation = second.population, secondPopulation > 0 else {
            return true
        }
        return firstPopulation < secondPopulation * 5
    }

    private static func normalized(_ value: String) -> String {
        value.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "en_US_POSIX"))
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func locationLabel(_ place: GeocodedPlace) -> String {
        var pieces = [place.name]
        if let admin1 = place.admin1, normalized(admin1) != normalized(place.name) {
            pieces.append(admin1)
        }
        if let country = place.country ?? place.countryCode, !pieces.contains(where: { normalized($0) == normalized(country) }) {
            pieces.append(country)
        }
        return pieces.joined(separator: ", ")
    }

    private static func format(place: GeocodedPlace, current: CurrentConditions, timezone: String, daily: DailyForecast, days: Int, forecastURL: URL) -> String {
        var lines = [
            "Weather for \(locationLabel(place))",
            "Timezone: \(timezone)",
            "Current observation: \(current.time) (\(timezone))",
            "Temperature: \(temperature(current.temperature2m)) °C; feels like \(temperature(current.apparentTemperature)) °C",
            "Condition: \(condition(current.weatherCode))",
            "Precipitation: \(measurement(current.precipitation, unit: "mm"))",
            "Wind: \(measurement(current.windSpeed10m, unit: "km/h")), direction \(direction(current.windDirection10m))"
        ]

        let availableDays = min(days, daily.time.count)
        for index in 0..<availableDays {
            let date = daily.time[index]
            let condition = value(daily.weatherCode, at: index).map(weatherDescription) ?? "unavailable"
            let high = temperature(value(daily.temperature2mMax, at: index), unit: "°C")
            let low = temperature(value(daily.temperature2mMin, at: index), unit: "°C")
            let chance = value(daily.precipitationProbabilityMax, at: index).map { "\(Self.number($0))%" } ?? "unavailable"
            let total = measurement(value(daily.precipitationSum, at: index), unit: "mm")
            lines.append("\(date): \(condition); high \(high), low \(low); precipitation chance \(chance); total \(total)")
        }

        lines.append("Source: Open-Meteo forecast (\(forecastURL.absoluteString)); weather data CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/). Docs: https://open-meteo.com/en/docs. Forecast models come from national meteorological services; location data is based on GeoNames.")
        return String(lines.joined(separator: "\n").prefix(5_000))
    }

    private static func value<Value>(_ values: [Value?]?, at index: Int) -> Value? {
        guard let values, values.indices.contains(index) else { return nil }
        return values[index]
    }

    private static func temperature(_ value: Double?, unit: String = "") -> String {
        guard let value else { return "unavailable" }
        let suffix = unit.isEmpty ? "" : " \(unit)"
        return "\(number(value))\(suffix)"
    }

    private static func measurement(_ value: Double?, unit: String) -> String {
        guard let value else { return "unavailable" }
        return "\(number(value)) \(unit)"
    }

    private static func direction(_ value: Double?) -> String {
        guard let value else { return "unavailable" }
        return "\(number(value))°"
    }

    private static func number(_ value: Double) -> String {
        String(format: "%.1f", locale: Locale(identifier: "en_US_POSIX"), value)
    }

    private static func condition(_ code: Int?) -> String {
        guard let code else { return "unavailable" }
        return "\(weatherDescription(code)) (WMO \(code))"
    }

    private static func weatherDescription(_ code: Int) -> String {
        switch code {
        case 0: return "Clear sky"
        case 1: return "Mainly clear"
        case 2: return "Partly cloudy"
        case 3: return "Overcast"
        case 45: return "Fog"
        case 48: return "Depositing rime fog"
        case 51: return "Light drizzle"
        case 53: return "Moderate drizzle"
        case 55: return "Dense drizzle"
        case 56: return "Light freezing drizzle"
        case 57: return "Dense freezing drizzle"
        case 61: return "Slight rain"
        case 63: return "Moderate rain"
        case 65: return "Heavy rain"
        case 66: return "Light freezing rain"
        case 67: return "Heavy freezing rain"
        case 71: return "Slight snow"
        case 73: return "Moderate snow"
        case 75: return "Heavy snow"
        case 77: return "Snow grains"
        case 80: return "Slight rain showers"
        case 81: return "Moderate rain showers"
        case 82: return "Violent rain showers"
        case 85: return "Slight snow showers"
        case 86: return "Heavy snow showers"
        case 95: return "Thunderstorm"
        case 96: return "Thunderstorm with slight hail"
        case 99: return "Thunderstorm with heavy hail"
        default: return "Unknown weather"
        }
    }

    private static let usStateAbbreviations: Set<String> = Set("AL AK AZ AR CA CO CT DE FL GA HI ID IL IN IA KS KY LA ME MD MA MI MN MS MO MT NE NV NH NJ NM NY NC ND OH OK OR PA RI SC SD TN TX UT VT VA WA WV WI WY DC PR".split(separator: " ").map(String.init))
}

private struct GeocodingQuery {
    let city: String
    let name: String
    let countryCode: String?
}

private struct GeocodingResponse: Decodable {
    let results: [GeocodedPlace]?
}

private struct GeocodedPlace: Decodable {
    let name: String
    let latitude: Double
    let longitude: Double
    let country: String?
    let countryCode: String?
    let admin1: String?
    let featureCode: String?
    let population: Int?

    enum CodingKeys: String, CodingKey {
        case name, latitude, longitude, country, admin1, population
        case countryCode = "country_code"
        case featureCode = "feature_code"
    }
}

private struct ForecastResponse: Decodable {
    let timezone: String
    let current: CurrentConditions?
    let daily: DailyForecast
}

private struct CurrentConditions: Decodable {
    let time: String
    let temperature2m: Double?
    let apparentTemperature: Double?
    let precipitation: Double?
    let weatherCode: Int?
    let windSpeed10m: Double?
    let windDirection10m: Double?

    enum CodingKeys: String, CodingKey {
        case time, precipitation
        case temperature2m = "temperature_2m"
        case apparentTemperature = "apparent_temperature"
        case weatherCode = "weather_code"
        case windSpeed10m = "wind_speed_10m"
        case windDirection10m = "wind_direction_10m"
    }
}

private struct DailyForecast: Decodable {
    let time: [String]
    let weatherCode: [Int?]?
    let temperature2mMax: [Double?]?
    let temperature2mMin: [Double?]?
    let precipitationProbabilityMax: [Double?]?
    let precipitationSum: [Double?]?

    enum CodingKeys: String, CodingKey {
        case time
        case weatherCode = "weather_code"
        case temperature2mMax = "temperature_2m_max"
        case temperature2mMin = "temperature_2m_min"
        case precipitationProbabilityMax = "precipitation_probability_max"
        case precipitationSum = "precipitation_sum"
    }
}

private struct APIErrorResponse: Decodable {
    let error: Bool?
    let reason: String?
}

enum LocalAssistantWeatherError: LocalizedError {
    case invalidLocation
    case invalidRequest
    case invalidResponse
    case noLocations(String)
    case apiFailure(statusCode: Int?, reason: String?)
    case responseTooLarge
    case malformedResponse

    var errorDescription: String? {
        switch self {
        case .invalidLocation:
            return "Enter a city name with at least two characters. Add a country or state if needed."
        case .invalidRequest:
            return "The weather request could not be created."
        case .invalidResponse:
            return "Open-Meteo returned an unexpected response."
        case .noLocations(let location):
            return "No locations matched \(location.debugDescription). Add a country or state, or check the spelling."
        case .apiFailure(let statusCode, let reason):
            let status = statusCode.map { " (HTTP \($0))" } ?? ""
            let details = reason.map { ": \(String($0.prefix(240)))" } ?? ""
            return "Open-Meteo request failed\(status)\(details)."
        case .responseTooLarge:
            return "Open-Meteo response exceeded the allowed size."
        case .malformedResponse:
            return "Open-Meteo returned invalid weather data."
        }
    }
}
