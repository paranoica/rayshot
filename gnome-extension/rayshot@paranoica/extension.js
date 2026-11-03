import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {pickClickPoint} from './geometry.js';

const RAYSHOT_BIN = GLib.build_filenamev([
    GLib.get_home_dir(),
    'projects/rayshot/target/release/rayshot',
]);
const ARM_TIMEOUT_MS = 5000;
const KILL_GRACE_MS = 1000;
const SETTLE_INTERVAL_MS = 20;
const SETTLE_ATTEMPTS = 25;
const SIGTERM = 15;
const POPUP_TYPES = [
    Meta.WindowType.DROPDOWN_MENU,
    Meta.WindowType.POPUP_MENU,
    Meta.WindowType.COMBO,
];
const ACTION_MODES = Shell.ActionMode.ALL &
    ~(Shell.ActionMode.LOGIN_SCREEN | Shell.ActionMode.LOCK_SCREEN | Shell.ActionMode.UNLOCK_SCREEN);

function trace(t0, what) {
    const ms = (GLib.get_monotonic_time() - t0) / 1000;
    console.log(`rayshot: +${ms.toFixed(1)}ms ${what}`);
}

function removeSource(id) {
    if (id)
        GLib.source_remove(id);
    return 0;
}

export default class RayshotExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._busy = false;
        this._overlay = null;
        this._armed = null;
        this._frameWatches = [];
        this._settleId = 0;

        Main.wm.addKeybinding(
            'capture',
            this._settings,
            Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
            ACTION_MODES,
            () => this._capture()
        );
        this._winCreatedId = global.display.connect('window-created',
            (_d, win) => this._onWindowCreated(win));
    }

    disable() {
        Main.wm.removeKeybinding('capture');
        global.display.disconnect(this._winCreatedId);
        this._winCreatedId = null;
        this._resetCapture();
        try {
            this._overlay?.proc.send_signal(SIGTERM);
        } catch (e) {
            logError(e, 'rayshot: cannot stop overlay');
        }
        this._overlay = null;
        this._settings = null;
    }

    _resetCapture() {
        this._clearFrameWatches();
        if (this._armed) {
            removeSource(this._armed.timeoutId);
            this._armed = null;
        }
        this._settleId = removeSource(this._settleId);
        if (this._overlay)
            this._overlay.killId = removeSource(this._overlay.killId);
    }

    _capture() {
        if (this._busy) {
            console.log('rayshot: capture already in progress, ignored');
            return;
        }
        this._busy = true;
        try {
            this._startCapture();
        } catch (e) {
            logError(e, 'rayshot: capture failed');
            this._busy = false;
        }
    }

    _startCapture() {
        const t0 = GLib.get_monotonic_time();
        const dir = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'rayshot']);
        GLib.mkdir_with_parents(dir, 0o700);
        const file = Gio.File.new_for_path(
            GLib.build_filenamev([dir, `frame-${t0}.png`]));
        const stream = file.replace(null, false, Gio.FileCreateFlags.PRIVATE, null);

        const shooter = new Shell.Screenshot();
        shooter.screenshot(false, stream, (_o, res) => {
            try {
                this._onScreenshot(shooter, res, stream, file, t0);
            } catch (e) {
                logError(e, 'rayshot: capture failed');
                this._discard(file);
                this._busy = false;
            }
        });
    }

    _onScreenshot(shooter, res, stream, file, t0) {
        let ok = true;
        try {
            shooter.screenshot_finish(res);
        } catch (e) {
            logError(e, 'rayshot: screenshot failed');
            ok = false;
        }
        try {
            stream.close(null);
        } catch (e) {
            logError(e, 'rayshot: cannot close frame file');
            ok = false;
        }
        if (!ok || !this._settings) {
            this._discard(file);
            this._busy = false;
            return;
        }
        trace(t0, 'frame captured');
        this._closeShellChrome();
        this._spawnOverlay(file, t0);
    }

    _spawnOverlay(file, t0) {
        let proc;
        try {
            proc = Gio.Subprocess.new([RAYSHOT_BIN, 'overlay', file.get_path()],
                Gio.SubprocessFlags.NONE);
        } catch (e) {
            logError(e, 'rayshot: spawn overlay failed');
            this._discard(file);
            this._busy = false;
            return;
        }
        const overlay = {proc, pid: Number(proc.get_identifier()), shown: false, killId: 0};
        this._overlay = overlay;
        this._arm(t0);
        proc.wait_async(null, () => {
            overlay.killId = removeSource(overlay.killId);
            if (this._overlay !== overlay)
                return;
            this._overlay = null;
            this._busy = false;
            this._resetCapture();
        });
    }

    _discard(file) {
        try {
            file.delete(null);
        } catch (_) {}
    }

    _closeShellChrome() {
        Main.layoutManager.emit('system-modal-opened');
        if (Main.overview.visible)
            Main.overview.hide();
    }

    _arm(t0) {
        if (this._armed)
            removeSource(this._armed.timeoutId);
        const timeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, ARM_TIMEOUT_MS, () => {
            this._armed = null;
            const overlay = this._overlay;
            if (overlay && !overlay.shown) {
                trace(t0, 'overlay showed no frame in time, terminating it');
                this._terminate(overlay);
            } else {
                trace(t0, 'no overlay first frame in time, grab left alone');
            }
            return GLib.SOURCE_REMOVE;
        });
        this._armed = {t0, timeoutId};
    }

    _terminate(overlay) {
        overlay.proc.send_signal(SIGTERM);
        overlay.killId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, KILL_GRACE_MS, () => {
            overlay.killId = 0;
            overlay.proc.force_exit();
            return GLib.SOURCE_REMOVE;
        });
    }

    _clearFrameWatches() {
        for (const [actor, ...ids] of this._frameWatches) {
            for (const id of ids)
                actor.disconnect(id);
        }
        this._frameWatches = [];
    }

    _dropFrameWatch(actor) {
        for (const [a, ...ids] of this._frameWatches) {
            if (a === actor) {
                for (const id of ids)
                    actor.disconnect(id);
            }
        }
        this._frameWatches = this._frameWatches.filter(([a]) => a !== actor);
    }

    _isOverlay(win) {
        return this._overlay !== null && win.get_pid() === this._overlay.pid;
    }

    _onWindowCreated(win) {
        if (!this._isOverlay(win))
            return;
        const overlay = this._overlay;
        if (this._armed)
            trace(this._armed.t0, `overlay window on monitor ${win.get_monitor()}`);
        const actor = win.get_compositor_private();
        if (!actor) {
            overlay.shown = true;
            Main.activateWindow(win);
            return;
        }
        const frameId = actor.connect('first-frame', () => {
            overlay.shown = true;
            this._dropFrameWatch(actor);
            Main.activateWindow(win);
            this._breakGrab(win);
        });
        const destroyId = actor.connect('destroy', () => {
            this._frameWatches = this._frameWatches.filter(([a]) => a !== actor);
        });
        this._frameWatches.push([actor, frameId, destroyId]);
    }

    _breakGrab(win) {
        if (!this._armed)
            return;
        const {t0, timeoutId} = this._armed;
        removeSource(timeoutId);
        this._armed = null;
        trace(t0, 'overlay first frame');

        const windows = global.get_window_actors()
            .map(a => a.get_meta_window())
            .filter(w => w);
        console.log(`rayshot: window types [${windows.map(w =>
            `${w.get_window_type()}/${w.get_client_type()}`).join(',')}]`);
        const popups = windows
            .filter(w => POPUP_TYPES.includes(w.get_window_type()) &&
                w.get_client_type() === Meta.WindowClientType.WAYLAND &&
                !this._isOverlay(w))
            .map(w => w.get_frame_rect());
        if (popups.length === 0) {
            trace(t0, 'no wayland popup, no click');
            return;
        }

        const area = Main.layoutManager.getWorkAreaForMonitor(win.get_monitor());
        const target = pickClickPoint(area, popups);
        const rects = popups.map(r => `${r.x},${r.y} ${r.width}x${r.height}`).join('; ');
        if (!target) {
            trace(t0, `popups [${rects}] cover every click point, no click`);
            return;
        }
        this._clickWhenSettled(win, this._overlay, target, rects, t0, 0);
    }

    _clickWhenSettled(win, overlay, target, rects, t0, attempt) {
        if (this._overlay !== overlay)
            return;
        const actor = win.get_compositor_private();
        const settled = actor && actor.opacity === 255 &&
            actor.scale_x === 1 && actor.scale_y === 1;
        let onTop = false;
        if (settled) {
            let hit = global.stage.get_actor_at_pos(Clutter.PickMode.REACTIVE, target.x, target.y);
            while (hit && !(hit instanceof Meta.WindowActor))
                hit = hit.get_parent();
            onTop = hit !== null && this._isOverlay(hit.get_meta_window());
        }
        if (!onTop) {
            if (attempt >= SETTLE_ATTEMPTS || !actor) {
                const why = settled
                    ? `target ${target.x},${target.y} is not over the overlay`
                    : 'overlay never settled';
                trace(t0, `popups [${rects}], ${why}, no click`);
                return;
            }
            this._settleId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, SETTLE_INTERVAL_MS, () => {
                this._settleId = 0;
                this._clickWhenSettled(win, overlay, target, rects, t0, attempt + 1);
                return GLib.SOURCE_REMOVE;
            });
            return;
        }

        const [x, y] = global.get_pointer();
        const seat = global.stage.context.get_backend().get_default_seat();
        const ptr = seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
        ptr.notify_absolute_motion(Clutter.CURRENT_TIME, target.x, target.y);
        ptr.notify_button(Clutter.CURRENT_TIME, Clutter.BUTTON_MIDDLE, Clutter.ButtonState.PRESSED);
        ptr.notify_button(Clutter.CURRENT_TIME, Clutter.BUTTON_MIDDLE, Clutter.ButtonState.RELEASED);
        ptr.notify_absolute_motion(Clutter.CURRENT_TIME, x, y);
        trace(t0, `popups [${rects}], clicked ${target.x},${target.y} after ${attempt} waits, pointer back to ${x},${y}`);
    }
}
