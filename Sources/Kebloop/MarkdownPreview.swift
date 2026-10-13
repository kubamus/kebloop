import Foundation
import SwiftUI

enum MarkdownBlock: Equatable {
    case heading(level: Int, text: String)
    case paragraph(String)
    case list(ordered: Bool, start: Int, items: [String])
    case code(language: String, contents: String)
    case quote(String)
    case rule
}

enum MarkdownParser {
    static func parse(_ source: String) -> [MarkdownBlock] {
        let lines = source.replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .components(separatedBy: "\n")
        var blocks: [MarkdownBlock] = []
        var index = 0

        while index < lines.count {
            let line = lines[index]
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.isEmpty {
                index += 1
                continue
            }

            if let fence = openingFence(in: trimmed) {
                index += 1
                var codeLines: [String] = []
                while index < lines.count {
                    if isClosingFence(lines[index].trimmingCharacters(in: .whitespaces), fence: fence) {
                        index += 1
                        break
                    }
                    codeLines.append(lines[index])
                    index += 1
                }
                blocks.append(.code(language: fence.language, contents: codeLines.joined(separator: "\n")))
                continue
            }

            if let heading = heading(in: trimmed) {
                blocks.append(.heading(level: heading.level, text: heading.text))
                index += 1
                continue
            }

            if isHorizontalRule(trimmed) {
                blocks.append(.rule)
                index += 1
                continue
            }

            if let item = listItem(in: trimmed) {
                let ordered = item.ordered
                let start = item.number ?? 1
                var items: [String] = []
                while index < lines.count,
                      let nextItem = listItem(in: lines[index].trimmingCharacters(in: .whitespaces)),
                      nextItem.ordered == ordered {
                    items.append(nextItem.text)
                    index += 1
                }
                blocks.append(.list(ordered: ordered, start: start, items: items))
                continue
            }

            if trimmed.hasPrefix(">") {
                var quoteLines: [String] = []
                while index < lines.count {
                    let quoteLine = lines[index].trimmingCharacters(in: .whitespaces)
                    guard quoteLine.hasPrefix(">") else { break }
                    quoteLines.append(String(quoteLine.dropFirst()).trimmingCharacters(in: .whitespaces))
                    index += 1
                }
                blocks.append(.quote(quoteLines.joined(separator: "\n")))
                continue
            }

            var paragraphLines: [String] = []
            while index < lines.count {
                let current = lines[index]
                let currentTrimmed = current.trimmingCharacters(in: .whitespaces)
                guard !currentTrimmed.isEmpty,
                      openingFence(in: currentTrimmed) == nil,
                      heading(in: currentTrimmed) == nil,
                      !isHorizontalRule(currentTrimmed),
                      listItem(in: currentTrimmed) == nil,
                      !currentTrimmed.hasPrefix(">") else { break }
                paragraphLines.append(current.trimmingCharacters(in: .whitespaces))
                index += 1
            }
            if !paragraphLines.isEmpty {
                blocks.append(.paragraph(paragraphLines.joined(separator: "\n")))
            } else {
                // Make progress for unsupported standalone Markdown syntax.
                blocks.append(.paragraph(trimmed))
                index += 1
            }
        }

        return blocks
    }

    private static func heading(in line: String) -> (level: Int, text: String)? {
        let level = line.prefix(while: { $0 == "#" }).count
        guard (1...6).contains(level),
              line.dropFirst(level).first?.isWhitespace == true else { return nil }
        return (level, String(line.dropFirst(level)).trimmingCharacters(in: .whitespaces))
    }

    private static func isHorizontalRule(_ line: String) -> Bool {
        let markers = line.filter { !$0.isWhitespace }
        guard markers.count >= 3, let marker = markers.first,
              marker == "-" || marker == "*" || marker == "_" else { return false }
        return markers.allSatisfy { $0 == marker }
    }

    private static func listItem(in line: String) -> (ordered: Bool, number: Int?, text: String)? {
        let characters = Array(line)
        guard let first = characters.first else { return nil }

        if ["-", "+", "*"].contains(first), characters.count > 1, characters[1].isWhitespace {
            return (false, nil, String(characters.dropFirst(2)).trimmingCharacters(in: .whitespaces))
        }

        var digitCount = 0
        while digitCount < characters.count, characters[digitCount].isNumber { digitCount += 1 }
        guard digitCount > 0, digitCount + 2 < characters.count,
              [".", ")"].contains(characters[digitCount]),
              characters[digitCount + 1].isWhitespace else { return nil }
        let number = Int(String(characters[..<digitCount]))
        return (
            true,
            number,
            String(characters.dropFirst(digitCount + 2)).trimmingCharacters(in: .whitespaces)
        )
    }

    private static func openingFence(in line: String) -> (marker: Character, count: Int, language: String)? {
        guard let marker = line.first, marker == "`" || marker == "~" else { return nil }
        let count = line.prefix(while: { $0 == marker }).count
        guard count >= 3 else { return nil }
        let language = String(line.dropFirst(count)).trimmingCharacters(in: .whitespaces)
        return (marker, count, language)
    }

    private static func isClosingFence(
        _ line: String,
        fence: (marker: Character, count: Int, language: String)
    ) -> Bool {
        let markerCount = line.prefix(while: { $0 == fence.marker }).count
        guard markerCount >= fence.count else { return false }
        return line.dropFirst(markerCount).allSatisfy(\.isWhitespace)
    }
}

enum MarkdownInlineParser {
    static func attributedString(from source: String) -> AttributedString {
        do {
            return try AttributedString(
                markdown: source,
                options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)
            )
        } catch {
            return AttributedString(source)
        }
    }
}

struct MarkdownPreview: View {
    let source: String

    private var blocks: [MarkdownBlock] { MarkdownParser.parse(source) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                    rendered(block)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 2)
        }
    }

    @ViewBuilder
    private func rendered(_ block: MarkdownBlock) -> some View {
        switch block {
        case let .heading(level, text):
            Text(MarkdownInlineParser.attributedString(from: text))
                .font(headingFont(level))
                .frame(maxWidth: .infinity, alignment: .leading)

        case let .paragraph(text):
            Text(MarkdownInlineParser.attributedString(from: text))
                .font(.body)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)

        case let .list(ordered, start, items):
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(ordered ? "\(start + index)." : "•")
                            .foregroundStyle(.secondary)
                            .frame(minWidth: ordered ? 24 : 12, alignment: .trailing)
                        Text(MarkdownInlineParser.attributedString(from: item))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
            .padding(.leading, 8)

        case let .code(language, contents):
            VStack(alignment: .leading, spacing: 6) {
                if !language.isEmpty {
                    Text(language)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                ScrollView(.horizontal) {
                    Text(contents)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(10)
            .background(Color(nsColor: .textBackgroundColor).opacity(0.65), in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.quaternary, lineWidth: 1))

        case let .quote(text):
            HStack(spacing: 10) {
                RoundedRectangle(cornerRadius: 1)
                    .fill(.secondary.opacity(0.45))
                    .frame(width: 3)
                Text(MarkdownInlineParser.attributedString(from: text))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .fixedSize(horizontal: false, vertical: true)
            .padding(.leading, 4)

        case .rule:
            Rectangle()
                .fill(Color.secondary.opacity(0.35))
                .frame(height: 1)
                .padding(.vertical, 4)
        }
    }

    private func headingFont(_ level: Int) -> Font {
        switch level {
        case 1: .system(size: 28, weight: .bold)
        case 2: .system(size: 23, weight: .bold)
        case 3: .system(size: 19, weight: .semibold)
        case 4: .system(size: 16, weight: .semibold)
        default: .system(size: 14, weight: .semibold)
        }
    }
}
