import AppKit
import SwiftUI
import XCTest
@testable import Kebloop

@MainActor
final class MarkdownNoteEditorTests: XCTestCase {
    func testAttachingNativeEditorFocusesTextAndStartsAtTheEnd() async throws {
        _ = NSApplication.shared
        var source = "# Existing note\n\nContinue here"
        let binding = Binding(get: { source }, set: { source = $0 })
        let host = NSHostingView(rootView: MarkdownNoteEditor(text: binding, focusRevision: 1, findRequest: nil))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 480, height: 320),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        defer { window.close() }
        host.layoutSubtreeIfNeeded()
        for _ in 0..<50 {
            if window.firstResponder is MarkdownNoteTextView { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        let editor = try XCTUnwrap(window.firstResponder as? MarkdownNoteTextView)
        XCTAssertEqual(editor.string, source)
        XCTAssertEqual(editor.selectedRange(), NSRange(location: (source as NSString).length, length: 0))
        XCTAssertTrue(editor.allowsUndo)
        XCTAssertFalse(editor.isRichText)
        let original = source
        editor.insertText("!", replacementRange: editor.selectedRange())
        XCTAssertEqual(source, original + "!", "Typing must update the bound draft without clicking first")
        XCTAssertTrue(editor.undoManager?.canUndo == true)
        editor.breakUndoCoalescing()
        try await Task.sleep(for: .milliseconds(20))
        editor.undoManager?.undo()
        XCTAssertEqual(editor.string, original)
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertEqual(source, original, "Native undo must update the bound draft too")
        editor.undoManager?.redo()
        XCTAssertEqual(source, original + "!", "Native redo must update the bound draft too")
    }

    func testFindRequestOpensNativeFindBarAfterAttachment() async throws {
        _ = NSApplication.shared
        let binding = Binding<String>.constant("Find this word")
        let host = NSHostingView(rootView: MarkdownNoteEditor(text: binding, focusRevision: 1,
            findRequest: MarkdownNoteFindRequest(revision: 1, action: .show)))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 480, height: 320),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        defer { window.close() }
        host.layoutSubtreeIfNeeded()
        for _ in 0..<50 {
            if descendants(host).contains(where: { ($0 as? NSScrollView)?.isFindBarVisible == true }) { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(descendants(host).contains(where: { ($0 as? NSScrollView)?.isFindBarVisible == true }),
                      "Find must work when requested before the editor is attached to its window")
    }

    private func descendants(_ view: NSView) -> [NSView] {
        [view] + view.subviews.flatMap(descendants)
    }
}
