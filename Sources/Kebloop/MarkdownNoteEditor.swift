import AppKit
import SwiftUI

struct MarkdownNoteEditor: NSViewRepresentable {
    @Binding var text: String
    let focusRevision: Int
    let findRequest: MarkdownNoteFindRequest?

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder

        let textView = MarkdownNoteTextView(frame: .zero)
        textView.delegate = context.coordinator
        textView.isRichText = false
        textView.importsGraphics = false
        textView.usesFindBar = true
        textView.allowsUndo = true
        textView.drawsBackground = false
        textView.textColor = .labelColor
        textView.font = .monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        textView.textContainerInset = NSSize(width: 12, height: 12)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.minSize = NSSize(width: 0, height: 0)
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.containerSize = NSSize(
            width: scrollView.contentSize.width,
            height: CGFloat.greatestFiniteMagnitude
        )
        textView.setAccessibilityRole(.textArea)
        textView.setAccessibilityLabel(L("Markdown note editor"))
        textView.onWindowReady = { [weak coordinator = context.coordinator, weak textView] in
            guard let coordinator, let textView else { return }
            coordinator.windowReady(textView)
        }

        scrollView.documentView = textView
        context.coordinator.requestFocus(in: textView, revision: focusRevision, force: true)
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.parent = self
        guard let textView = scrollView.documentView as? MarkdownNoteTextView else { return }

        if textView.string != text, !textView.hasMarkedText() {
            let selection = textView.selectedRange()
            textView.string = text
            let location = min(selection.location, (text as NSString).length)
            let length = min(selection.length, (text as NSString).length - location)
            textView.setSelectedRange(NSRange(location: location, length: length))
        }
        context.coordinator.requestFocus(in: textView, revision: focusRevision)
        if let findRequest { context.coordinator.performFind(findRequest, in: textView) }
    }

    static func dismantleNSView(_ scrollView: NSScrollView, coordinator: Coordinator) {
        guard let textView = scrollView.documentView as? MarkdownNoteTextView else { return }
        textView.onWindowReady = nil
        textView.removeWindowObserver()
        coordinator.removeUndoObservers()
        textView.delegate = nil
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: MarkdownNoteEditor
        private var lastFocusedRevision: Int?
        private var lastFindRevision: Int?
        private var scheduledFindRevision: Int?
        private weak var observedUndoManager: UndoManager?
        private var undoObservers: [NSObjectProtocol] = []

        init(_ parent: MarkdownNoteEditor) { self.parent = parent }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView,
                  !textView.hasMarkedText() else { return }
            parent.text = textView.string
        }

        func requestFocus(in textView: MarkdownNoteTextView, revision: Int, force: Bool = false) {
            guard force || lastFocusedRevision != revision else { return }
            DispatchQueue.main.async { [weak self, weak textView] in
                guard let self, let textView, let window = textView.window else { return }
                guard self.parent.focusRevision == revision else { return }
                if window.firstResponder !== textView {
                    guard window.makeFirstResponder(textView) else { return }
                    textView.setSelectedRange(NSRange(location: (textView.string as NSString).length, length: 0))
                }
                self.lastFocusedRevision = revision
            }
        }

        func windowReady(_ textView: MarkdownNoteTextView) {
            if let manager = textView.undoManager, observedUndoManager !== manager {
                removeUndoObservers()
                observedUndoManager = manager
                undoObservers = [Notification.Name.NSUndoManagerDidUndoChange, Notification.Name.NSUndoManagerDidRedoChange].map { name in
                    NotificationCenter.default.addObserver(forName: name, object: manager, queue: .main) { [weak self, weak textView] _ in
                        guard let self, let textView, !textView.hasMarkedText() else { return }
                        self.parent.text = textView.string
                    }
                }
            }
            requestFocus(in: textView, revision: parent.focusRevision)
            if let findRequest = parent.findRequest { performFind(findRequest, in: textView) }
        }

        func removeUndoObservers() {
            undoObservers.forEach(NotificationCenter.default.removeObserver)
            undoObservers.removeAll()
            observedUndoManager = nil
        }

        deinit { undoObservers.forEach(NotificationCenter.default.removeObserver) }

        func performFind(_ request: MarkdownNoteFindRequest, in textView: MarkdownNoteTextView) {
            guard lastFindRevision != request.revision,
                  scheduledFindRevision != request.revision else { return }
            scheduledFindRevision = request.revision
            DispatchQueue.main.async { [weak self, weak textView] in
                guard let self else { return }
                guard let textView, textView.window != nil else {
                    if self.scheduledFindRevision == request.revision { self.scheduledFindRevision = nil }
                    return
                }
                guard self.parent.findRequest?.revision == request.revision else {
                    if self.scheduledFindRevision == request.revision { self.scheduledFindRevision = nil }
                    if let latestRequest = self.parent.findRequest { self.performFind(latestRequest, in: textView) }
                    return
                }
                if request.action == .show, textView.window?.firstResponder !== textView {
                    guard textView.window?.makeFirstResponder(textView) == true else {
                        self.scheduledFindRevision = nil
                        return
                    }
                }
                let item = NSMenuItem()
                item.tag = request.action.textFinderAction.rawValue
                textView.performTextFinderAction(item)
                self.lastFindRevision = request.revision
                self.scheduledFindRevision = nil
            }
        }
    }
}

final class MarkdownNoteTextView: NSTextView {
    var onWindowReady: (() -> Void)?
    private var keyObserver: NSObjectProtocol?

    override var acceptsFirstResponder: Bool { true }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        removeWindowObserver()
        guard let window else { return }
        keyObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.didBecomeKeyNotification,
            object: window,
            queue: .main
        ) { [weak self] _ in
            DispatchQueue.main.async { self?.onWindowReady?() }
        }
        DispatchQueue.main.async { [weak self] in self?.onWindowReady?() }
    }

    func removeWindowObserver() {
        if let keyObserver { NotificationCenter.default.removeObserver(keyObserver) }
        keyObserver = nil
    }

    deinit { removeWindowObserver() }
}

struct MarkdownNoteKeyMonitor: NSViewRepresentable {
    let onKeyEvent: (NSEvent) -> Bool

    func makeNSView(context: Context) -> MarkdownNoteKeyMonitorView {
        let view = MarkdownNoteKeyMonitorView()
        view.onKeyEvent = onKeyEvent
        return view
    }

    func updateNSView(_ view: MarkdownNoteKeyMonitorView, context: Context) {
        view.onKeyEvent = onKeyEvent
    }

    static func dismantleNSView(_ view: MarkdownNoteKeyMonitorView, coordinator: ()) {
        view.removeEventMonitor()
        view.onKeyEvent = nil
    }
}

final class MarkdownNoteKeyMonitorView: NSView {
    var onKeyEvent: ((NSEvent) -> Bool)?
    private var eventMonitor: Any?

    override var intrinsicContentSize: NSSize { NSSize(width: 1, height: 1) }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        removeEventMonitor()
        guard let window else { return }
        eventMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self, weak window] event in
            guard let self, let window, event.window === window else { return event }
            if window.attachedSheet != nil { return event }
            if let textView = window.firstResponder as? NSTextView, textView.hasMarkedText() { return event }
            let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            if event.keyCode == 53,
               modifiers.isDisjoint(with: [.command, .shift, .option, .control]),
               let textView = self.noteTextView(in: window.contentView),
               textView.enclosingScrollView?.isFindBarVisible == true {
                let item = NSMenuItem()
                item.tag = NSTextFinder.Action.hideFindInterface.rawValue
                textView.performTextFinderAction(item)
                return nil
            }
            return self.onKeyEvent?(event) == true ? nil : event
        }
    }

    private func noteTextView(in view: NSView?) -> MarkdownNoteTextView? {
        guard let view else { return nil }
        if let textView = view as? MarkdownNoteTextView { return textView }
        for child in view.subviews {
            if let textView = noteTextView(in: child) { return textView }
        }
        return nil
    }

    func removeEventMonitor() {
        if let eventMonitor { NSEvent.removeMonitor(eventMonitor) }
        eventMonitor = nil
    }

    deinit { removeEventMonitor() }
}
