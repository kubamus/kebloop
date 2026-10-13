import Foundation
import XCTest
@testable import Kebloop

final class MarkdownPreviewTests: XCTestCase {
    func testParserKeepsMarkdownBlocksSeparateForRenderedPreview() {
        let source = """
        # Project plan

        Read **the brief** first.

        - Draft the outline
        - Share it

        3. Review
        4. Publish

        ```swift
        let answer = 42
        ```

        > Keep this short.
        """

        XCTAssertEqual(MarkdownParser.parse(source), [
            .heading(level: 1, text: "Project plan"),
            .paragraph("Read **the brief** first."),
            .list(ordered: false, start: 1, items: ["Draft the outline", "Share it"]),
            .list(ordered: true, start: 3, items: ["Review", "Publish"]),
            .code(language: "swift", contents: "let answer = 42"),
            .quote("Keep this short.")
        ])
    }

    func testInlineMarkdownRetainsClickableLinkAttribute() throws {
        let attributed = MarkdownInlineParser.attributedString(from: "Read [the guide](https://example.com/guide).")
        let link = try XCTUnwrap(attributed.runs.compactMap(\.link).first)

        XCTAssertEqual(link.absoluteString, "https://example.com/guide")
        XCTAssertTrue(String(attributed.characters).contains("the guide"))
    }

    func testParserAcceptsUnclosedFencesAndNormalizesWindowsLineEndings() {
        XCTAssertEqual(
            MarkdownParser.parse("## Notes\r\n\r\n```text\r\nline one\r\nline two"),
            [
                .heading(level: 2, text: "Notes"),
                .code(language: "text", contents: "line one\nline two")
            ]
        )
    }

    func testParserRecognizesHorizontalRulesInsteadOfShowingTheirMarkdown() {
        XCTAssertEqual(
            MarkdownParser.parse("Before\n\n---\n\n***\n\n___\n\nAfter"),
            [
                .paragraph("Before"),
                .rule,
                .rule,
                .rule,
                .paragraph("After")
            ]
        )
        XCTAssertEqual(MarkdownParser.parse("--"), [.paragraph("--")])
    }
}
