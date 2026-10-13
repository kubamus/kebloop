import Gio from 'gi://Gio';
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const XML = `<node><interface name="org.kebloop.Gnome1">
<method name="Ping"><arg type="u" direction="out"/></method>
<method name="Windows"><arg type="s" direction="out"/></method>
<method name="Outputs"><arg type="s" direction="out"/></method>
<method name="Command"><arg type="s" direction="in"/></method>
<method name="GetClipboard"><arg type="b" direction="in"/><arg type="s" direction="out"/></method>
<method name="GetClipboardData"><arg type="s" direction="out"/></method>
<method name="SetClipboard"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="b" direction="in"/></method>
<method name="ClearClipboard"/>
<method name="Paste"><arg type="b" direction="in"/></method>
</interface></node>`;
const MAX_BYTES = 8 * 1024 * 1024;

export default class KebloopIntegration extends Extension {
    enable() {
        this._revision = 0;
        this._epoch = 0;
        this._recording = false;
        this._ownSource = null;
        this._clipboard = {revision: 0, mime_type: null, data: null, source_app: null};
        this._selection = global.display.get_selection();
        this._signal = this._selection.connect('owner-changed', (_selection, type, owner) => {
            if (type === Meta.SelectionType.SELECTION_CLIPBOARD)
                this._readClipboard(true, owner !== null && owner === this._ownSource);
        });
        this._object = Gio.DBusExportedObject.wrapJSObject(XML, this);
        this._object.export(Gio.DBus.session, '/org/kebloop/Gnome');
        this._bus = Gio.bus_own_name_on_connection(Gio.DBus.session,
            'org.kebloop.Gnome', Gio.BusNameOwnerFlags.NONE, null, null);
        this._readClipboard();
    }

    disable() {
        this._cancel?.cancel();
        if (this._deadline)
            GLib.source_remove(this._deadline);
        this._deadline = 0;
        this._selection.disconnect(this._signal);
        this._object.unexport();
        Gio.bus_unown_name(this._bus);
        this._selection = null;
        this._keyboard?.run_dispose();
        this._keyboard = null;
    }

    Ping() { return 1; }

    _windows() {
        return global.get_window_actors().map(actor => actor.meta_window)
            .filter(window => !window.is_override_redirect() && !window.is_skip_taskbar());
    }

    Windows() {
        const tracker = Shell.WindowTracker.get_default();
        return JSON.stringify(this._windows().map(window => {
            const frame = window.get_frame_rect();
            return {
                id: String(window.get_stable_sequence()),
                app_id: window.get_gtk_application_id() ?? window.get_wm_class()
                    ?? tracker.get_window_app(window)?.get_id(),
                title: window.get_title() ?? '', pid: window.get_pid(),
                focused: window.has_focus(), floating: true,
                fullscreen: window.is_fullscreen(), hidden: window.minimized,
                output: String(window.get_monitor()),
                workspace: String(window.get_workspace()?.index() ?? -1),
                frame: {x: frame.x, y: frame.y, w: frame.width, h: frame.height},
                size: [frame.width, frame.height], focus_rank: window.get_user_time(),
            };
        }));
    }

    Outputs() {
        const active = global.display.focus_window?.get_monitor();
        const workspace = global.workspace_manager.get_active_workspace();
        return JSON.stringify(Main.layoutManager.monitors.map((monitor, index) => {
            const usable = workspace.get_work_area_for_monitor(index);
            return {name: String(index),
                frame: {x: monitor.x, y: monitor.y, w: monitor.width, h: monitor.height},
                usable: {x: usable.x, y: usable.y, w: usable.width, h: usable.height},
                focused: active === index};
        }));
    }

    Command(json) {
        const command = JSON.parse(json);
        const window = this._windows().find(candidate =>
            String(candidate.get_stable_sequence()) === command.id);
        if (!window)
            throw new Error('Window no longer exists');
        const time = global.get_current_time();
        switch (command.action) {
        case 'focus':
            window.unminimize();
            Main.activateWindow(window, time);
            break;
        case 'close': window.delete(time); break;
        case 'hide': window.minimize(); break;
        case 'frame': {
            const {x, y, w, h} = command.frame;
            if (![x, y, w, h].every(Number.isInteger) || w < 1 || h < 1)
                throw new Error('Invalid frame');
            window.unmake_fullscreen();
            window.unmaximize(Meta.MaximizeFlags.BOTH);
            window.move_resize_frame(true, x, y, w, h);
            break;
        }
        default: throw new Error('Unsupported command');
        }
    }

    GetClipboard(recording) {
        if (recording !== this._recording) {
            this._recording = recording;
            this._readClipboard(false);
        }
        return JSON.stringify({...this._clipboard, data: {}});
    }
    GetClipboardData() { return JSON.stringify(this._clipboard); }
    SetClipboard(mime, encoded, transient) {
        if (encoded.length > Math.ceil(MAX_BYTES / 3) * 4)
            throw new Error('Clipboard payload too large');
        const bytes = GLib.base64_decode(encoded);
        if (bytes.length > MAX_BYTES)
            throw new Error('Clipboard payload too large');
        this._ownPending = true;
        this._ownTransient = transient;
        this._ownSource = Meta.SelectionSourceMemory.new(mime, new GLib.Bytes(bytes));
        this._selection.set_owner(Meta.SelectionType.SELECTION_CLIPBOARD, this._ownSource);
        this._ownPending = false;
    }
    Paste(shift) {
        const backend = global.stage.context?.get_backend() ?? Clutter.get_default_backend();
        this._keyboard ??= backend.get_default_seat()
            .create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        if (!this._keyboard)
            throw new Error('Virtual keyboard unavailable');
        const keys = shift ? [Clutter.KEY_Control_L, Clutter.KEY_Shift_L, Clutter.KEY_v]
            : [Clutter.KEY_Control_L, Clutter.KEY_v];
        try {
            for (const key of keys)
                this._keyboard.notify_keyval(GLib.get_monotonic_time(), key, Clutter.KeyState.PRESSED);
        } finally {
            for (const key of keys.reverse())
                this._keyboard.notify_keyval(GLib.get_monotonic_time(), key, Clutter.KeyState.RELEASED);
        }
    }
    ClearClipboard() {
        this._ownSource = Meta.SelectionSourceMemory.new('text/plain', new GLib.Bytes(new Uint8Array()));
        this._selection.set_owner(Meta.SelectionType.SELECTION_CLIPBOARD, this._ownSource);
        this._selection.unset_owner(Meta.SelectionType.SELECTION_CLIPBOARD, this._ownSource);
        this._ownSource = null;
    }

    _readClipboard(changed = true, own = false) {
        const epoch = ++this._epoch;
        this._cancel?.cancel();
        if (this._deadline)
            GLib.source_remove(this._deadline);
        this._deadline = 0;
        const revision = changed ? ++this._revision : this._revision;
        this._clipboard = {revision, mime_type: null, data: null, source_app: null};
        const type = Meta.SelectionType.SELECTION_CLIPBOARD;
        const types = this._selection.get_mimetypes(type) ?? [];
        this._clipboard = {revision, mimes: types, data: {}, source_app: null,
            own_write: own, sensitive: types.some(mime =>
                /concealed|transient|autogenerated|passwordmanagerhint/i.test(mime)), ready: false};
        if (!this._recording || this._clipboard.sensitive || (own && this._ownTransient)) {
            this._clipboard.ready = true;
            return;
        }
        const preferred = ['text/plain;charset=utf-8', 'text/plain', 'text/html',
            'text/rtf', 'image/png', 'text/uri-list'].filter(mime => types.includes(mime));
        if (preferred.includes('text/plain;charset=utf-8'))
            preferred.splice(preferred.indexOf('text/plain'), preferred.includes('text/plain') ? 1 : 0);
        const cancel = new Gio.Cancellable();
        this._cancel = cancel;
        this._deadline = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 2000, () => {
            this._deadline = 0;
            cancel.cancel();
            if (epoch === this._epoch)
                this._clipboard.ready = true;
            return GLib.SOURCE_REMOVE;
        });
        let pending = preferred.length;
        if (pending === 0) {
            this._clipboard.ready = true;
            GLib.source_remove(this._deadline);
            this._deadline = 0;
        }
        for (const mime of preferred) {
            const stream = Gio.MemoryOutputStream.new_resizable();
            this._selection.transfer_async(type, mime, MAX_BYTES + 1, stream, cancel,
                (selection, result) => {
                    try {
                        selection.transfer_finish(result);
                        stream.close(null);
                        if (epoch !== this._epoch || cancel.is_cancelled())
                            return;
                        if (stream.get_data_size() <= MAX_BYTES)
                            this._clipboard.data[mime] = GLib.base64_encode(stream.steal_as_bytes().get_data());
                    } catch (_) {
                        // Failed/cancelled reads retain the immediately published revision.
                    } finally {
                        if (--pending === 0 && epoch === this._epoch) {
                            this._clipboard.ready = true;
                            if (this._deadline)
                                GLib.source_remove(this._deadline);
                            this._deadline = 0;
                        }
                    }
                });
        }
    }
}
