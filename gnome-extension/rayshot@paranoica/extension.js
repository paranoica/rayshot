import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {frameHeader} from './frame.js';
import {pickClickPoint} from './geometry.js';

const RAYSHOT_BIN = GLib.build_filenamev([
    GLib.get_home_dir(),
    'projects/rayshot/target/release/rayshot',
]);
const ARM_TIMEOUT_MS = 5000;
const KILL_GRACE_MS = 1000;
const PAINT_FALLBACK_MS = 50;
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

function frameDir() {
    return GLib.build_filenamev([GLib.get_user_runtime_dir(), 'rayshot']);
}

function deleteQuietly(path) {
    try {
        Gio.File.new_for_path(path).delete(null);
    } catch (_) {}
}

function sweepFrames() {
    let entries;
    try {
        entries = Gio.File.new_for_path(frameDir()).enumerate_children(
            'standard::name', Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS, null);
    } catch (_) {
        return;
    }
    try {
        for (let info; (info = entries.next_file(null));) {
            const name = info.get_name();
            if (/^frame-\d+\.(rsf|png)$/.test(name))
                deleteQuietly(GLib.build_filenamev([frameDir(), name]));
        }
    } catch (e) {
        logError(e, 'rayshot: cannot sweep old frames');
    } finally {
        entries.close(null);
    }
}

function removeSource(id) {
    if (id)
        GLib.source_remove(id);
    return 0;
}

function typeName(obj) {
    return obj.constructor.$gtype?.name ?? '';
}

function surfaceOf(windowActor) {
    return windowActor.get_children().find(c => typeName(c).startsWith('MetaSurface')) ??
        windowActor.get_last_child();
}

export default class RayshotExtension extends Extension {
    enable() {
        this._epoch = (this._epoch ?? 0) + 1;
        sweepFrames();
        this._settings = this.getSettings();
        this._busy = false;
        this._overlay = null;
        this._armed = null;
        this._frozen = null;
        this._frameWatches = [];
        this._settleId = 0;
        this._paintWait = null;

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
        this._epoch++;
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
        this._unfreeze();
        this._settings = null;
    }

    _resetCapture() {
        this._clearFrameWatches();
        if (this._armed) {
            removeSource(this._armed.timeoutId);
            this._armed = null;
        }
        this._settleId = removeSource(this._settleId);
        this._cancelPaintWait();
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
            this._abort();
        }
    }

    _abort() {
        this._unfreeze();
        this._busy = false;
    }

    _startCapture() {
        const t0 = GLib.get_monotonic_time();
        const epoch = this._epoch;
        const shooter = new Shell.Screenshot();
        shooter.screenshot_stage_to_content((_o, res) => {
            if (epoch !== this._epoch)
                return;
            try {
                this._onStageContent(shooter, res, t0);
            } catch (e) {
                logError(e, 'rayshot: capture failed');
                this._abort();
            }
        });
    }

    _onStageContent(shooter, res, t0) {
        const [content] = shooter.screenshot_stage_to_content_finish(res);
        if (this._frozen) {
            console.log('rayshot: screen already frozen, capture dropped');
            return;
        }
        const texture = content.get_texture();
        this._freezeScreen(content, texture);
        trace(t0, `screen frozen ${texture.get_width()}x${texture.get_height()}`);
        this._closeShellChrome();
        this._afterNextPaint(() => this._launch(texture, t0));
    }

    _freezeScreen(content, texture) {
        const actor = new Clutter.Actor({
            content,
            x: 0,
            y: 0,
            width: global.stage.width,
            height: global.stage.height,
            reactive: true,
        });
        Main.layoutManager.uiGroup.add_child(actor);
        St.Settings.get().inhibit_animations();
        global.compositor.disable_unredirect();
        this._frozen = {actor, texture, clones: []};
    }

    _unfreeze() {
        const freeze = this._frozen;
        if (!freeze)
            return;
        this._frozen = null;
        freeze.clones.forEach(c => c.destroy());
        freeze.actor.destroy();
        St.Settings.get().uninhibit_animations();
        global.compositor.enable_unredirect();
    }

    _afterNextPaint(fn) {
        this._cancelPaintWait();
        const wait = {paintId: 0, fallbackId: 0, idleId: 0};
        const run = () => {
            if (this._paintWait !== wait)
                return;
            this._cancelPaintWait();
            fn();
        };
        wait.paintId = global.stage.connect('after-paint', () => {
            if (!wait.idleId) {
                wait.idleId = GLib.idle_add(GLib.PRIORITY_HIGH, () => {
                    wait.idleId = 0;
                    run();
                    return GLib.SOURCE_REMOVE;
                });
            }
        });
        wait.fallbackId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, PAINT_FALLBACK_MS, () => {
            wait.fallbackId = 0;
            run();
            return GLib.SOURCE_REMOVE;
        });
        this._paintWait = wait;
    }

    _cancelPaintWait() {
        const wait = this._paintWait;
        if (!wait)
            return;
        this._paintWait = null;
        if (wait.paintId)
            global.stage.disconnect(wait.paintId);
        removeSource(wait.fallbackId);
        removeSource(wait.idleId);
    }

    _launch(texture, t0) {
        if (!this._settings || this._frozen?.texture !== texture)
            return;
        let proc;
        try {
            proc = Gio.Subprocess.new(
                [RAYSHOT_BIN, 'overlay', '--live', `${texture.get_width()}`, `${texture.get_height()}`],
                Gio.SubprocessFlags.STDIN_PIPE);
        } catch (e) {
            logError(e, 'rayshot: spawn overlay failed');
            this._abort();
            return;
        }
        trace(t0, 'overlay spawned');
        const overlay = {
            proc,
            pid: Number(proc.get_identifier()),
            shown: false,
            killId: 0,
            framePath: null,
        };
        this._overlay = overlay;
        this._arm(t0);
        proc.wait_async(null, () => {
            overlay.killId = removeSource(overlay.killId);
            if (overlay.framePath)
                deleteQuietly(overlay.framePath);
            if (this._overlay !== overlay)
                return;
            this._overlay = null;
            this._busy = false;
            this._resetCapture();
            this._unfreeze();
        });
        this._sendPixels(overlay, texture, t0);
    }

    _sendPixels(overlay, texture, t0) {
        const sink = Gio.File.new_for_path('/dev/null').append_to(Gio.FileCreateFlags.NONE, null);
        Shell.Screenshot.composite_to_stream(texture, 0, 0, -1, -1, 1, null, 0, 0, 1, sink, (_o, res) => {
            try {
                sink.close(null);
                const pixbuf = Shell.Screenshot.composite_to_stream_finish(res);
                if (this._overlay !== overlay)
                    return;
                overlay.framePath = this._writeFrame(pixbuf, t0);
                overlay.proc.get_stdin_pipe().write_all(
                    new TextEncoder().encode(`frame ${overlay.framePath}\n`), null);
                trace(t0, 'pixels sent');
            } catch (e) {
                logError(e, 'rayshot: cannot hand pixels to the overlay');
                if (overlay.framePath)
                    deleteQuietly(overlay.framePath);
                if (this._overlay === overlay)
                    this._terminate(overlay);
            }
        });
    }

    _writeFrame(pixbuf, t0) {
        if (pixbuf.get_n_channels() !== 4 || pixbuf.get_bits_per_sample() !== 8)
            throw new Error(`unexpected pixbuf layout ${pixbuf.get_n_channels()}x${pixbuf.get_bits_per_sample()}`);
        const started = GLib.get_monotonic_time();
        const dir = frameDir();
        GLib.mkdir_with_parents(dir, 0o700);
        const file = Gio.File.new_for_path(GLib.build_filenamev([dir, `frame-${t0}.rsf`]));
        const out = file.replace(null, false, Gio.FileCreateFlags.PRIVATE, null);
        let written = false;
        try {
            out.write_all(frameHeader(pixbuf.get_width(), pixbuf.get_height(), pixbuf.get_rowstride()), null);
            const pixels = pixbuf.read_pixel_bytes();
            const size = pixels.get_size();
            for (let off = 0; off < size;) {
                const chunk = off === 0 ? pixels : GLib.Bytes.new_from_bytes(pixels, off, size - off);
                const n = out.write_bytes(chunk, null);
                if (n <= 0)
                    throw new Error(`short write at ${off} of ${size}`);
                off += n;
            }
            written = true;
        } finally {
            out.close(null);
            if (!written)
                deleteQuietly(file.get_path());
        }
        trace(started, `frame file written ${pixbuf.get_width()}x${pixbuf.get_height()}`);
        return file.get_path();
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
        if (overlay.killId)
            return;
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
            console.log('rayshot: overlay window has no actor, cannot show it over the freeze');
            this._terminate(overlay);
            return;
        }
        const frameId = actor.connect('first-frame', () => {
            this._dropFrameWatch(actor);
            if (!this._showThroughFreeze(actor)) {
                console.log('rayshot: no overlay surface to show over the freeze');
                this._terminate(overlay);
                return;
            }
            overlay.shown = true;
            Main.activateWindow(win);
            this._breakGrab(win);
        });
        const destroyId = actor.connect('destroy', () => {
            this._frameWatches = this._frameWatches.filter(([a]) => a !== actor);
        });
        this._frameWatches.push([actor, frameId, destroyId]);
    }

    _showThroughFreeze(windowActor) {
        const freeze = this._frozen;
        if (!freeze)
            return false;
        const surface = surfaceOf(windowActor);
        if (!surface)
            return false;
        const clone = new Clutter.Clone({source: surface, reactive: false});
        clone.add_constraint(new Clutter.BindConstraint({
            source: windowActor,
            coordinate: Clutter.BindCoordinate.POSITION,
        }));
        Main.layoutManager.uiGroup.insert_child_above(clone, freeze.actor);
        freeze.clones.push(clone);
        freeze.actor.reactive = false;
        return true;
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
