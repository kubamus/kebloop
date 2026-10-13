// Tests bridge state transitions without GNOME; real Shell smoke tests still required.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
const transfers = [];
const selection = {
    types: [], get_mimetypes() { return this.types; },
    transfer_async(_type, mime, max, stream, cancel, callback) {
        transfers.push({mime, max, stream, cancel, callback});
    },
    transfer_finish() {},
};
let nextTimer = 0;
const context = vm.createContext({
    Extension: class {}, global: {}, Main: {}, Shell: {}, Clutter: {},
    Meta: {SelectionType: {SELECTION_CLIPBOARD: 1}},
    GLib: {timeout_add: () => ++nextTimer, source_remove() {},
        base64_encode: bytes => Buffer.from(bytes).toString('base64')},
    Gio: {
        Cancellable: class { cancelled = false; cancel() { this.cancelled = true; } is_cancelled() { return this.cancelled; } },
        MemoryOutputStream: {new_resizable: () => ({bytes: Buffer.from('text'), close() {}, get_data_size() { return this.bytes.length; }, steal_as_bytes() { return {get_data: () => this.bytes}; }})},
    },
});
const source = readFileSync(new URL('./kebloop@kebloop.app/extension.js', import.meta.url), 'utf8')
    .replace(/^import .*;$/gm, '').replace('export default class KebloopIntegration', 'globalThis.Bridge = class KebloopIntegration');
vm.runInContext(source, context);
const bridge = new context.Bridge();
Object.assign(bridge, {_revision: 0, _epoch: 0, _recording: false, _selection: selection});
selection.types = ['text/plain', 'image/png'];
bridge._readClipboard();
assert.equal(bridge._clipboard.revision, 1);
assert.equal(transfers.length, 0, 'paused watcher must not read payload');
bridge.GetClipboard(true);
assert.equal(transfers.length, 2);
assert.equal(bridge._clipboard.revision, 1, 'recording toggle is not an owner change');
assert.equal(bridge._clipboard.ready, false);
assert.deepEqual(JSON.parse(bridge.GetClipboard(true)).data, {}, 'metadata excludes payload');
selection.types = [];
bridge._readClipboard();
assert.equal(bridge._clipboard.revision, 2, 'clear revision is immediate');
assert.equal(transfers[0].cancel.is_cancelled(), true);
for (const transfer of transfers)
    transfer.callback(selection, {});
assert.deepEqual(Object.keys(bridge._clipboard.data), [], 'stale callbacks cannot overwrite new selection');
selection.types = ['text/plain', 'x-kde-passwordManagerHint'];
bridge._readClipboard();
assert.equal(bridge._clipboard.sensitive, true);
assert.equal(transfers.length, 2, 'sensitive offered marker prevents reads');
selection.types = ['text/plain'];
bridge._readClipboard();
assert.equal(transfers.at(-1).max, 8 * 1024 * 1024 + 1);
transfers.at(-1).callback(selection, {});
assert.equal(bridge._clipboard.ready, true);
assert.equal(JSON.parse(bridge.GetClipboardData()).data['text/plain'], Buffer.from('text').toString('base64'));
assert.equal(bridge._clipboard.source_app, null);
console.log('GNOME bridge state tests passed');
