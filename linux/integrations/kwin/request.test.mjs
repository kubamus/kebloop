import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import assert from 'node:assert/strict';
import test from 'node:test';
const source = readFileSync(new URL('./request.js', import.meta.url), 'utf8');
function execute(request, workspace) {
    let reply;
    runInNewContext(source, {request: {...request, callback: ':1.2'}, workspace,
        KWin: {MaximizeArea: 2}, callDBus(service, path, iface, method, json) {
            assert.equal(service, ':1.2');
            assert.equal(method, 'Complete');
            reply = JSON.parse(json);
        }});
    return reply;
}
function fixture() {
    const desktop = {id: 'desktop-1'};
    const screen = {name: 'DP-1', geometry: {x: 0, y: 0, width: 1920, height: 1080}};
    const window = {internalId: 'window-1', desktopFileName: 'org.kde.konsole', resourceClass: 'konsole',
        caption: 'Terminal', pid: 100, desktops: [desktop], activities: ['work'], output: screen,
        minimized: true, fullScreen: false, frameGeometry: {x: 100, y: 100, width: 800, height: 600},
        minimizable: true, closeable: true, moveable: true, resizeable: true,
        setMaximize(vertical, horizontal) { this.maximized = [vertical, horizontal]; },
        closeWindow() { this.closed = true; }};
    const workspace = {stackingOrder: [window], screens: [screen], currentDesktop: desktop,
        activeWindow: null, activeScreen: screen, currentActivity: 'other',
        clientArea() {return {x: 0, y: 30, width: 1920, height: 1050};}};
    return {window, workspace};
}
test('snapshots include hidden applications and omit panels', () => {
    const {window, workspace} = fixture();
    workspace.stackingOrder.push({specialWindow: true});
    const reply = execute({action: 'windows'}, workspace);
    assert.equal(reply.ok, true);
    assert.equal(reply.value.length, 1);
    assert.equal(reply.value[0].hidden, true);
    assert.equal(reply.value[0].id, window.internalId);
    assert.equal(reply.value[0].app_id, 'org.kde.konsole');
    assert.deepEqual(reply.value[0].frame, {x: 100, y: 100, w: 800, h: 600});
});
test('focusing restores a minimized window and activates its desktop/activity', () => {
    const {window, workspace} = fixture();
    assert.equal(execute({action: 'focus', id: 'window-1'}, workspace).ok, true);
    assert.equal(window.minimized, false);
    assert.equal(workspace.activeWindow, window);
    assert.equal(workspace.currentActivity, 'work');
});
test('frame request clears maximization and applies frame geometry', () => {
    const {window, workspace} = fixture();
    assert.equal(execute({action: 'frame', id: 'window-1', frame: {x: -1920, y: 30, w: 960, h: 1050}}, workspace).ok, true);
    assert.deepEqual(window.maximized, [false, false]);
    assert.equal(window.tile, null);
    assert.equal(window.frameGeometry.x, -1920);
    assert.equal(window.frameGeometry.width, 960);
});
test('outputs report panel-adjusted workareas', () => {
    const {workspace} = fixture();
    const reply = execute({action: 'outputs'}, workspace);
    assert.deepEqual(reply.value[0].usable, {x: 0, y: 30, w: 1920, h: 1050});
});
test('invalid or vanished windows report rejection', () => {
    const {workspace} = fixture();
    assert.equal(execute({action: 'close', id: 'gone'}, workspace).ok, false);
    assert.equal(execute({action: 'invalid', id: 'window-1'}, workspace).ok, false);
});
test('close and hide respect window permissions', () => {
    const {window, workspace} = fixture();
    window.minimizable = false;
    assert.equal(execute({action: 'hide', id: 'window-1'}, workspace).ok, false);
    assert.equal(execute({action: 'close', id: 'window-1'}, workspace).ok, true);
    assert.equal(window.closed, true);
});
