// Request is supplied as JSON by the native backend, never interpolated as code.
// KWin 6 API: https://develop.kde.org/docs/plasma/kwin/api/
(function () {
    function rect(r) { return {x: r.x, y: r.y, w: r.width, h: r.height}; }
    function clients() { return workspace.stackingOrder; }
    function execute() {
        if (request.action === "ping") return 1;
        if (request.action === "windows") {
            return clients().filter(function (w) {
                return !w.deleted && !w.specialWindow && !w.skipTaskbar;
            }).map(function (w) {
                return {id: String(w.internalId), app_id: String(w.desktopFileName || w.resourceClass),
                    title: w.caption, pid: w.pid, focused: w === workspace.activeWindow,
                    fullscreen: w.fullScreen, hidden: w.minimized, frame: rect(w.frameGeometry),
                    output: w.output ? w.output.name : null,
                    workspace: w.desktops.length ? String(w.desktops[0].id) : null};
            });
        }
        if (request.action === "outputs") {
            return workspace.screens.map(function (screen) {
                return {name: screen.name, frame: rect(screen.geometry),
                    usable: rect(workspace.clientArea(KWin.MaximizeArea, screen, workspace.currentDesktop)),
                    focused: screen === workspace.activeScreen};
            });
        }
        var window = clients().filter(function (w) { return String(w.internalId) === request.id; })[0];
        if (!window || window.deleted) throw new Error("window no longer exists");
        switch (request.action) {
        case "focus":
        case "show":
            window.minimized = false;
            if (window.desktops.length) workspace.currentDesktop = window.desktops[0];
            if (window.activities.length && window.activities.indexOf(workspace.currentActivity) < 0)
                workspace.currentActivity = window.activities[0];
            workspace.activeWindow = window;
            break;
        case "hide":
            if (!window.minimizable) throw new Error("window cannot be minimized");
            window.minimized = true;
            break;
        case "close":
            if (!window.closeable) throw new Error("window cannot be closed");
            window.closeWindow();
            break;
        case "frame":
            if (!window.moveable || !window.resizeable) throw new Error("window cannot be moved or resized");
            window.fullScreen = false;
            window.setMaximize(false, false);
            window.tile = null;
            window.frameGeometry = {x: request.frame.x, y: request.frame.y,
                width: request.frame.w, height: request.frame.h};
            break;
        default: throw new Error("unsupported window command");
        }
        return null;
    }
    var result;
    try { result = {ok: true, value: execute()}; }
    catch (error) { result = {ok: false, error: String(error)}; }
    callDBus(request.callback, "/org/kebloop/KWinReply", "org.kebloop.KWinReply1", "Complete", JSON.stringify(result));
})();
