import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import System from 'system';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as MessageTray from 'resource:///org/gnome/shell/ui/messageTray.js';

import {MAX_FAILURES, respawnDelay, shouldRespawn} from './backoff.js';
import {PROTOCOL_VERSION, defaultSeat, monitorManager, persistentNotice, readyVersion, resolveBin, unredirect} from './compat.js';
import {frameHeader} from './frame.js';
import {frameMonitors, pickClickPoint} from './geometry.js';

const ARM_TIMEOUT_MS = 5000;
const DRAG_WATCH_MS = 50;
const PROTOCOL_MISMATCH_EXIT = 3;
const UPDATED_TEXT = 'rayshot was updated. Log out and back in to finish the update.';
const BINARY_OLD_TEXT = 'The rayshot program is older than its GNOME extension. Update rayshot.';
const SPARE_FAILED_TEXT = 'rayshot keeps failing to prepare its overlay, so captures open slower. ' +
    'Run "rayshot warm off" and then "rayshot warm on" to try again.';
const READY_TIMEOUT_MS = 10000;
const STABLE_SERVER_MS = 10000;
const WINDOWS_GONE_TIMEOUT_MS = 500;
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

function parentPid(pid) {
    try {
        const stat = new TextDecoder().decode(GLib.file_get_contents(`/proc/${pid}/stat`)[1]);
        return Number(stat.slice(stat.lastIndexOf(')') + 2).split(' ')[1]);
    } catch (_) {
        return 0;
    }
}

function spawnedBy(pid, ancestor) {
    for (let i = 0; pid > 1 && i < 8; i++) {
        pid = parentPid(pid);
        if (pid === ancestor)
            return true;
    }
    return false;
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
        this._givenUp ??= {failures: 0, notified: false};
        sweepFrames();
        this._settings = this.getSettings();
        this._debug = this._settings.get_boolean('debug-log');
        this._debugId = this._settings.connect('changed::debug-log', () => {
            this._debug = this._settings.get_boolean('debug-log');
        });
        this._connectors = [];
        this._outputsCancel = new Gio.Cancellable();
        this._readOutputs();
        this._monitorsId = Main.layoutManager.connect('monitors-changed', () => this._readOutputs());
        this._notified = new Set();
        this._copyNotice = null;
        this._log(`using ${this._bin() ?? 'no rayshot binary'}`);
        this._watchOwnFiles();
        this._busy = false;
        this._overlay = null;
        this._armed = null;
        this._frozen = null;
        this._frameWatches = [];
        this._settleId = 0;
        this._paintWait = null;
        this._session = 0;
        this._server = null;
        this._serverFailures = this._givenUp.failures;
        if (this._givenUp.notified)
            this._notified.add('spare-failed');
        this._binMisses = 0;
        this._dying = new Set();
        this._respawnId = 0;
        this._gcId = 0;
        this._warmId = this._settings.connect('changed::warm-overlay', () => this._syncServer());
        this._syncServer(false);

        Main.wm.addKeybinding(
            'capture',
            this._settings,
            Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
            ACTION_MODES,
            () => this._capture()
        );
        this._winCreatedId = global.display.connect('window-created',
            (_d, win) => this._onWindowCreated(win));
        this._refocusId = 0;
        this._focusId = global.display.connect('notify::focus-window',
            () => this._keepOverlayFocused());
    }

    disable() {
        const keep = Main.sessionMode.isLocked && !shouldRespawn(this._serverFailures);
        this._givenUp = {
            failures: keep ? this._serverFailures : 0,
            notified: keep && this._notified.has('spare-failed'),
        };
        this._epoch++;
        Main.wm.removeKeybinding('capture');
        global.display.disconnect(this._winCreatedId);
        this._winCreatedId = null;
        global.display.disconnect(this._focusId);
        this._focusId = null;
        this._refocusId = removeSource(this._refocusId);
        this._resetCapture();
        if (this._overlay) {
            this._releaseWindows(this._overlay);
            this._cancelPixels(this._overlay);
        }
        try {
            this._overlay?.proc.send_signal(SIGTERM);
            this._overlay?.proc.force_exit();
        } catch (e) {
            logError(e, 'rayshot: cannot stop overlay');
        }
        if (this._overlay?.framePath)
            deleteQuietly(this._overlay.framePath);
        this._overlay = null;
        this._unfreeze();
        this._stopServer();
        for (const server of this._dying) {
            server.killId = removeSource(server.killId);
            server.proc.force_exit();
        }
        this._dying.clear();
        this._gcId = removeSource(this._gcId);
        this._settings.disconnect(this._warmId);
        this._settings.disconnect(this._debugId);
        Main.layoutManager.disconnect(this._monitorsId);
        this._outputsCancel.cancel();
        this._outputsCancel = null;
        this._selfMonitor?.cancel();
        this._selfMonitor = null;
        this._copyNotice?.destroy();
        this._copyNotice = null;
        this._settings = null;
    }

    _readOutputs() {
        const cancellable = this._outputsCancel;
        Gio.DBus.session.call('org.gnome.Mutter.DisplayConfig', '/org/gnome/Mutter/DisplayConfig',
            'org.gnome.Mutter.DisplayConfig', 'GetCurrentState', null, null, Gio.DBusCallFlags.NONE,
            -1, cancellable, (conn, res) => {
                try {
                    const [, , logical] = conn.call_finish(res).recursiveUnpack();
                    this._connectors = logical.flatMap(([, , , , , outputs]) =>
                        outputs.map(([connector]) => connector));
                } catch (e) {
                    if (!cancellable.is_cancelled())
                        logError(e, 'rayshot: cannot read the monitor layout');
                }
            });
    }

    _frameMonitors(texture) {
        const n = global.display.get_n_monitors();
        const geometries = [...Array(n).keys()].map(i => global.display.get_monitor_geometry(i));
        const manager = monitorManager(global, Meta);
        return frameMonitors(this._connectors, c => manager?.get_monitor_for_connector(c) ?? -1, geometries,
            {width: texture.get_width(), height: texture.get_height()},
            {width: global.stage.width, height: global.stage.height});
    }

    _log(what) {
        if (this._debug)
            console.log(`rayshot: ${what}`);
    }

    _trace(t0, what) {
        this._log(`+${((GLib.get_monotonic_time() - t0) / 1000).toFixed(1)}ms ${what}`);
    }

    _bin() {
        return resolveBin(GLib.getenv('RAYSHOT_BIN'), this._settings.get_string('binary'),
            GLib.find_program_in_path('rayshot'),
            path => GLib.file_test(path, GLib.FileTest.IS_EXECUTABLE));
    }

    _notifyOnce(key, body) {
        if (this._notified.has(key))
            return;
        this._notified.add(key);
        Main.notify('rayshot', body);
    }

    // A package upgrade replaces these files while the old code stays loaded
    // until the next login on Wayland.
    _watchOwnFiles() {
        try {
            const file = Gio.File.new_for_path(GLib.build_filenamev([this.path, 'extension.js']));
            this._selfMonitor = file.monitor_file(Gio.FileMonitorFlags.NONE, null);
            this._selfMonitor.connect('changed', (_m, _f, _o, event) => {
                if (event === Gio.FileMonitorEvent.CHANGES_DONE_HINT || event === Gio.FileMonitorEvent.CREATED)
                    this._notifyOnce('updated', UPDATED_TEXT);
            });
        } catch (e) {
            logError(e, 'rayshot: cannot watch for updates');
        }
    }

    _syncServer(fresh = true) {
        if (!this._settings.get_boolean('warm-overlay')) {
            this._stopServer();
            this._log('warm overlay off');
        } else if (!this._server && !this._respawnId) {
            if (fresh) {
                this._serverFailures = 0;
                this._notified.delete('spare-failed');
            }
            this._binMisses = 0;
            if (shouldRespawn(this._serverFailures))
                this._startServer();
            else
                console.log('rayshot: overlay server gave up earlier, run "rayshot warm off" and "on" to retry');
        }
    }

    _startServer() {
        this._respawnId = 0;
        if (!this._settings?.get_boolean('warm-overlay'))
            return;
        const epoch = this._epoch;
        const bin = this._bin();
        if (!bin) {
            this._notifyOnce('no-bin', 'The rayshot program was not found. Install rayshot or set the binary path.');
            // Kept apart from the failure count, so an install after login
            // starts with a clean slate.
            this._respawnId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, respawnDelay(this._binMisses++), () => {
                this._startServer();
                return GLib.SOURCE_REMOVE;
            });
            return;
        }
        this._binMisses = 0;
        let proc;
        try {
            proc = Gio.Subprocess.new([bin, 'serve'],
                Gio.SubprocessFlags.STDIN_PIPE | Gio.SubprocessFlags.STDOUT_PIPE);
        } catch (e) {
            logError(e, 'rayshot: cannot start the overlay server');
            this._scheduleRespawn();
            return;
        }
        const server = {
            proc,
            pid: Number(proc.get_identifier()),
            ready: false,
            served: false,
            showSent: false,
            startedAt: GLib.get_monotonic_time(),
            readyId: 0,
            killId: 0,
            cancellable: new Gio.Cancellable(),
            lines: new Gio.DataInputStream({base_stream: proc.get_stdout_pipe()}),
        };
        server.readyId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, READY_TIMEOUT_MS, () => {
            server.readyId = 0;
            console.log('rayshot: overlay server not ready in time, restarting it');
            this._killServer(server);
            return GLib.SOURCE_REMOVE;
        });
        this._server = server;
        this._readServer(server);
        proc.wait_async(null, () => this._onServerExit(server, epoch));
    }

    _stopServer() {
        this._respawnId = removeSource(this._respawnId);
        const server = this._server;
        if (!server)
            return;
        this._server = null;
        server.cancellable.cancel();
        this._killServer(server);
    }

    _killServer(server) {
        server.ready = false;
        server.readyId = removeSource(server.readyId);
        if (server.killId)
            return;
        try {
            server.proc.send_signal(SIGTERM);
        } catch (e) {
            logError(e, 'rayshot: cannot stop the overlay server');
        }
        this._dying.add(server);
        server.killId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, KILL_GRACE_MS, () => {
            server.killId = 0;
            this._dying.delete(server);
            server.proc.force_exit();
            return GLib.SOURCE_REMOVE;
        });
    }

    _scheduleRespawn(served) {
        if (!this._settings?.get_boolean('warm-overlay'))
            return;
        let delay = 0;
        if (!served) {
            delay = respawnDelay(this._serverFailures);
            this._serverFailures++;
            if (!shouldRespawn(this._serverFailures)) {
                console.log(`rayshot: overlay server failed ${MAX_FAILURES} times in a row, not restarting it`);
                this._notifyOnce('spare-failed', SPARE_FAILED_TEXT);
                return;
            }
        }
        if (served)
            this._log('overlay server restarts');
        else
            console.log(`rayshot: overlay server restarts in ${delay}ms`);
        this._respawnId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, delay, () => {
            this._startServer();
            return GLib.SOURCE_REMOVE;
        });
    }

    _onCopyFailed(line) {
        const path = /^copy-failed (.+)$/.exec(line)?.[1];
        if (!path)
            return false;
        if (!this._settings)
            return true;
        // Main.notify is transient and would leave the tray after its banner;
        // an empty clipboard should still be explained when the user looks.
        // One notice at a time, so repeated failures do not crowd the tray.
        this._copyNotice?.destroy();
        const notice = persistentNotice(MessageTray, Main.messageTray, 'rayshot',
            `The screenshot could not be copied to the clipboard. A temporary copy is at ${path}.`);
        notice.connect('destroy', () => {
            if (this._copyNotice === notice)
                this._copyNotice = null;
        });
        this._copyNotice = notice;
        return true;
    }

    _readOneShot(lines) {
        lines.read_line_async(GLib.PRIORITY_DEFAULT, null, (stream, res) => {
            let line;
            try {
                [line] = stream.read_line_finish_utf8(res);
            } catch (e) {
                logError(e, 'rayshot: cannot read the overlay');
                return;
            }
            if (line === null)
                return;
            this._onCopyFailed(line);
            this._readOneShot(lines);
        });
    }

    _readServer(server) {
        server.lines.read_line_async(GLib.PRIORITY_DEFAULT, server.cancellable, (stream, res) => {
            let line;
            try {
                [line] = stream.read_line_finish_utf8(res);
            } catch (e) {
                if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    logError(e, 'rayshot: cannot read the overlay server');
                return;
            }
            if (line === null)
                return;
            this._onServerLine(server, line);
            this._readServer(server);
        });
    }

    _onServerLine(server, line) {
        if (this._onCopyFailed(line))
            return;
        if (this._server !== server)
            return;
        const version = readyVersion(line);
        if (version !== null) {
            server.readyId = removeSource(server.readyId);
            if (version !== PROTOCOL_VERSION) {
                console.log(`rayshot: overlay server speaks protocol ${version}, expected ${PROTOCOL_VERSION}`);
                if (version > PROTOCOL_VERSION)
                    this._notifyOnce('updated', UPDATED_TEXT);
                else
                    this._notifyOnce('binary-old', BINARY_OLD_TEXT);
                this._stopServer();
                return;
            }
            server.ready = true;
            this._log('overlay server ready');
            return;
        }
        const reply = /^(closed|failed) (\d+)$/.exec(line);
        const overlay = this._overlay;
        if (!reply || overlay?.server !== server || overlay.session !== Number(reply[2]))
            return;
        if (reply[1] === 'closed') {
            server.served = true;
            overlay.closed = true;
            this._serverFailures = 0;
            this._settleOverlay(overlay);
        } else if (overlay.shown || !this._frozen) {
            console.log(`rayshot: session ${overlay.session} failed on the server`);
            this._settleOverlay(overlay);
        } else {
            console.log(`rayshot: session ${overlay.session} failed before showing, falling back`);
            this._retryOneShot(overlay);
            this._killServer(server);
        }
    }

    _retryOneShot(overlay) {
        if (overlay.cancelled) {
            this._settleOverlay(overlay);
            return;
        }
        this._releaseWindows(overlay);
        this._cancelPixels(overlay);
        this._resetCapture();
        this._overlay = null;
        if (overlay.framePath)
            deleteQuietly(overlay.framePath);
        this._launch(overlay.texture, overlay.t0);
    }

    _onServerExit(server, epoch) {
        server.readyId = removeSource(server.readyId);
        server.killId = removeSource(server.killId);
        this._dying?.delete(server);
        if (server.ready && GLib.get_monotonic_time() - server.startedAt > STABLE_SERVER_MS * 1000)
            this._serverFailures = 0;
        // The reader runs on to EOF so a late `copy-failed` still lands, but a
        // late `closed` is dropped as stale; a clean exit counts as served.
        if (server.showSent && server.proc.get_if_exited() && server.proc.get_exit_status() === 0)
            server.served = true;
        const current = this._server === server && epoch === this._epoch;
        if (current) {
            this._server = null;
            if (!server.served)
                console.log(`rayshot: overlay server exited (status ${server.proc.get_status()})`);
            this._scheduleRespawn(server.served);
        }
        const overlay = this._overlay;
        if (overlay?.server === server) {
            if (!overlay.shown && !overlay.closed && !server.served && this._frozen)
                this._retryOneShot(overlay);
            else
                this._settleOverlay(overlay);
        }
    }

    // Unfreezing while an overlay window is still mapped would expose the
    // opaque backdrop mutter puts under fullscreen windows.
    _settleOverlay(overlay) {
        if (overlay.settleId || this._overlay !== overlay)
            return;
        if (overlay.windows.size === 0) {
            this._endOverlay(overlay);
            return;
        }
        overlay.settleId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, WINDOWS_GONE_TIMEOUT_MS, () => {
            overlay.settleId = 0;
            console.log(`rayshot: ${overlay.windows.size} overlay window(s) still mapped, unfreezing anyway`);
            this._endOverlay(overlay);
            return GLib.SOURCE_REMOVE;
        });
    }

    _trackWindow(overlay, win) {
        const id = win.connect('unmanaged', () => {
            win.disconnect(id);
            overlay.windows.delete(win);
            if (overlay.settleId && overlay.windows.size === 0) {
                overlay.settleId = removeSource(overlay.settleId);
                this._endOverlay(overlay);
            }
        });
        overlay.windows.set(win, id);
    }

    _releaseWindows(overlay) {
        overlay.settleId = removeSource(overlay.settleId);
        for (const [win, id] of overlay.windows)
            win.disconnect(id);
        overlay.windows.clear();
    }

    _endOverlay(overlay) {
        this._releaseWindows(overlay);
        this._cancelPixels(overlay);
        overlay.killId = removeSource(overlay.killId);
        if (overlay.framePath)
            deleteQuietly(overlay.framePath);
        if (this._overlay !== overlay)
            return;
        this._overlay = null;
        this._busy = false;
        this._resetCapture();
        this._unfreeze();
        this._collectSoon();
    }

    _collectSoon() {
        // Frozen textures and pixbufs are native memory the JS heap does not see.
        if (this._gcId)
            return;
        this._gcId = GLib.idle_add(GLib.PRIORITY_LOW, () => {
            this._gcId = 0;
            const started = GLib.get_monotonic_time();
            System.gc();
            this._trace(started, 'garbage collected');
            return GLib.SOURCE_REMOVE;
        });
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
            const windows = [...this._overlay?.windows.keys() ?? []];
            if (this._overlay?.shown && windows.length > 0) {
                windows.forEach(w => Main.activateWindow(w));
                this._log('capture already open, brought it to the front');
            } else {
                this._log('capture already in progress, ignored');
            }
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
        this._trace(t0, `screen frozen ${texture.get_width()}x${texture.get_height()}`);
        this._closeShellChrome();
        global.stage.set_key_focus(this._frozen.actor);
        if (this._server?.ready)
            this._showOnServer(this._server, texture, t0);
        else
            this._afterNextPaint(() => this._launch(texture, t0));
    }

    _showOnServer(server, texture, t0) {
        const overlay = {
            proc: server.proc,
            pid: server.pid,
            server,
            texture,
            t0,
            session: ++this._session,
            shown: false,
            killId: 0,
            settleId: 0,
            windows: new Map(),
            pixelIds: [],
            pixelPaintId: 0,
            framePath: null,
        };
        try {
            const show = ['show', overlay.session, texture.get_width(), texture.get_height(),
                ...this._frameMonitors(texture)].join(' ');
            server.proc.get_stdin_pipe().write_all(new TextEncoder().encode(`${show}\n`), null);
        } catch (e) {
            logError(e, 'rayshot: overlay server unreachable, falling back');
            this._afterNextPaint(() => this._launch(texture, t0));
            return;
        }
        server.ready = false;
        server.showSent = true;
        this._trace(t0, `session ${overlay.session} sent to the server`);
        this._overlay = overlay;
        this._arm(t0);
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
        // Until the overlay shows, keys would reach the app hidden under the
        // freeze; the freeze takes them instead, and Esc cancels.
        actor.connect('key-press-event', (_a, event) => {
            if (event.get_key_symbol() === Clutter.KEY_Escape)
                this._cancelCapture();
            return Clutter.EVENT_STOP;
        });
        // A drag that starts before the overlay shows lands on the freeze, which
        // keeps the pointer until release; the drag is forwarded as a selection.
        actor.connect('button-press-event', (_a, event) => {
            if (event.get_button() === Clutter.BUTTON_PRIMARY && this._frozen &&
                !this._frozen.drag?.held) {
                const [x, y] = event.get_coords();
                this._frozen.drag = {from: [x, y], to: [x, y], held: true, watchId: 0};
            }
            return Clutter.EVENT_STOP;
        });
        actor.connect('motion-event', (_a, event) => {
            const drag = this._frozen?.drag;
            if (drag?.held) {
                drag.to = event.get_coords();
                this._forwardDrag();
            }
            return Clutter.EVENT_STOP;
        });
        actor.connect('button-release-event', (_a, event) => {
            const drag = this._frozen?.drag;
            if (event.get_button() === Clutter.BUTTON_PRIMARY && drag?.held) {
                drag.to = event.get_coords();
                this._endDrag();
            }
            return Clutter.EVENT_STOP;
        });
        St.Settings.get().inhibit_animations();
        unredirect(global.compositor, global.display, Meta, false);
        const frozen = {actor, texture, clones: [], focusReleased: false};
        // A shell modal that ends after the overlay showed would hand the key
        // focus back to the freeze and leave the overlay without keys.
        frozen.focusId = global.stage.connect('notify::key-focus', () => {
            if (frozen.focusReleased && global.stage.get_key_focus() === actor)
                global.stage.set_key_focus(null);
        });
        this._frozen = frozen;
    }

    _forwardDrag() {
        const freeze = this._frozen;
        const drag = freeze?.drag;
        const overlay = this._overlay;
        if (!drag || !overlay?.shown || drag.forwardFailed)
            return;
        // The frame is in texture pixels, the events in stage units.
        const sx = freeze.texture.get_width() / global.stage.width;
        const sy = freeze.texture.get_height() / global.stage.height;
        const [x0, y0] = drag.from;
        const [x1, y1] = drag.to;
        const line = `select ${overlay.session} ${x0 * sx} ${y0 * sy} ${x1 * sx} ${y1 * sy}\n`;
        try {
            overlay.proc.get_stdin_pipe().write_all(new TextEncoder().encode(line), null);
        } catch (e) {
            drag.forwardFailed = true;
            logError(e, 'rayshot: cannot forward the selection');
        }
    }

    _endDrag() {
        const freeze = this._frozen;
        const drag = freeze?.drag;
        if (!drag)
            return;
        drag.held = false;
        drag.watchId = removeSource(drag.watchId);
        this._forwardDrag();
        if (this._overlay?.shown)
            freeze.actor.reactive = false;
        const retry = drag.afterRelease;
        drag.afterRelease = null;
        retry?.();
    }

    // Clutter normally reports the release, but the pointer must never stay
    // captured by the freeze if that event goes missing.
    _watchDrag() {
        const drag = this._frozen?.drag;
        if (!drag?.held || drag.watchId)
            return;
        drag.watchId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, DRAG_WATCH_MS, () => {
            const [x, y, mods] = global.get_pointer();
            if (this._frozen?.drag !== drag || !drag.held) {
                drag.watchId = 0;
                return GLib.SOURCE_REMOVE;
            }
            if (mods & Clutter.ModifierType.BUTTON1_MASK)
                return GLib.SOURCE_CONTINUE;
            drag.watchId = 0;
            drag.to = [x, y];
            this._endDrag();
            return GLib.SOURCE_REMOVE;
        });
    }

    _releaseKeyFocus() {
        if (!this._frozen)
            return;
        this._frozen.focusReleased = true;
        if (global.stage.get_key_focus() === this._frozen.actor)
            global.stage.set_key_focus(null);
    }

    _cancelCapture() {
        const overlay = this._overlay;
        if (!overlay) {
            this._log('capture cancelled before the overlay started');
            this._cancelPaintWait();
            this._abort();
            return;
        }
        overlay.cancelled = true;
        const line = overlay.server ? `cancel ${overlay.session}\n` : 'quit\n';
        try {
            overlay.proc.get_stdin_pipe().write_all(new TextEncoder().encode(line), null);
            this._log('capture cancelled before the overlay showed');
        } catch (e) {
            logError(e, 'rayshot: cannot cancel the overlay');
            this._terminate(overlay);
        }
    }

    _unfreeze() {
        const freeze = this._frozen;
        if (!freeze)
            return;
        this._frozen = null;
        global.stage.disconnect(freeze.focusId);
        if (freeze.drag)
            freeze.drag.watchId = removeSource(freeze.drag.watchId);
        freeze.clones.forEach(c => c.destroy());
        freeze.actor.destroy();
        St.Settings.get().uninhibit_animations();
        unredirect(global.compositor, global.display, Meta, true);
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
        const bin = this._bin();
        if (!bin) {
            this._notifyOnce('no-bin', 'The rayshot program was not found. Install rayshot or set the binary path.');
            this._abort();
            return;
        }
        let proc;
        try {
            proc = Gio.Subprocess.new(
                [bin, 'overlay', '--live', `${texture.get_width()}`, `${texture.get_height()}`,
                    '--protocol', `${PROTOCOL_VERSION}`, ...this._frameMonitors(texture)],
                Gio.SubprocessFlags.STDIN_PIPE | Gio.SubprocessFlags.STDOUT_PIPE);
        } catch (e) {
            logError(e, 'rayshot: spawn overlay failed');
            this._abort();
            return;
        }
        this._trace(t0, 'one-shot overlay spawned');
        this._readOneShot(new Gio.DataInputStream({base_stream: proc.get_stdout_pipe()}));
        const overlay = {
            proc,
            pid: Number(proc.get_identifier()),
            server: null,
            texture,
            t0,
            session: ++this._session,
            shown: false,
            killId: 0,
            settleId: 0,
            windows: new Map(),
            pixelIds: [],
            pixelPaintId: 0,
            framePath: null,
        };
        this._overlay = overlay;
        this._arm(t0);
        proc.wait_async(null, () => {
            if (proc.get_if_exited() && proc.get_exit_status() === PROTOCOL_MISMATCH_EXIT)
                this._notifyOnce('mismatch', 'rayshot and its GNOME extension do not match. Update rayshot, then log out and back in.');
            this._settleOverlay(overlay);
        });
    }

    _sendPixelsAfterPaint(overlay) {
        const send = () => {
            overlay.pixelIds.forEach(id => removeSource(id));
            if (overlay.pixelPaintId)
                global.stage.disconnect(overlay.pixelPaintId);
            overlay.pixelIds = [];
            overlay.pixelPaintId = 0;
            if (this._overlay === overlay)
                this._sendPixels(overlay, overlay.texture, overlay.t0);
            return GLib.SOURCE_REMOVE;
        };
        overlay.pixelPaintId = global.stage.connect('after-paint', () => {
            global.stage.disconnect(overlay.pixelPaintId);
            overlay.pixelPaintId = 0;
            overlay.pixelIds.push(GLib.idle_add(GLib.PRIORITY_HIGH, send));
        });
        overlay.pixelIds.push(GLib.timeout_add(GLib.PRIORITY_DEFAULT, PAINT_FALLBACK_MS, send));
    }

    _cancelPixels(overlay) {
        overlay.pixelIds.forEach(id => removeSource(id));
        overlay.pixelIds = [];
        if (overlay.pixelPaintId)
            global.stage.disconnect(overlay.pixelPaintId);
        overlay.pixelPaintId = 0;
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
                    new TextEncoder().encode(`frame ${overlay.session} ${overlay.framePath}\n`), null);
                this._trace(t0, 'pixels sent');
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
        this._trace(started, `frame file written ${pixbuf.get_width()}x${pixbuf.get_height()}`);
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
                console.log(`rayshot: overlay showed no frame in ${ARM_TIMEOUT_MS}ms, terminating it`);
                this._terminate(overlay);
            } else {
                console.log(`rayshot: no overlay first frame in ${ARM_TIMEOUT_MS}ms, grab left alone`);
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
        this._trackWindow(overlay, win);
        if (this._armed)
            this._trace(this._armed.t0, `overlay window on monitor ${win.get_monitor()}`);
        const actor = win.get_compositor_private();
        if (!actor) {
            console.log('rayshot: overlay window has no actor, cannot show it over the freeze');
            this._terminate(overlay);
            return;
        }
        const frameId = actor.connect('first-frame', () => {
            this._dropFrameWatch(actor);
            if (overlay.cancelled)
                return;
            if (!this._showThroughFreeze(actor)) {
                console.log('rayshot: no overlay surface to show over the freeze');
                this._terminate(overlay);
                return;
            }
            const first = !overlay.shown;
            overlay.shown = true;
            this._releaseKeyFocus();
            Main.activateWindow(win);
            this._breakGrab(win);
            // The texture readback blocks the compositor, so it waits until the
            // overlay has been painted instead of delaying it.
            if (first) {
                this._forwardDrag();
                this._sendPixelsAfterPaint(overlay);
            }
        });
        const destroyId = actor.connect('destroy', () => {
            this._frameWatches = this._frameWatches.filter(([a]) => a !== actor);
        });
        this._frameWatches.push([actor, frameId, destroyId]);
    }

    // An app focused under the freeze would take every key while hidden. The
    // overlay's own helpers are exempt: wl-copy needs the focus to copy. The
    // check waits a turn so windows of an ending session are already gone.
    _keepOverlayFocused() {
        if (this._refocusId || !this._overlay?.shown)
            return;
        this._refocusId = GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
            this._refocusId = 0;
            const overlay = this._overlay;
            const focus = global.display.focus_window;
            if (!overlay?.shown || overlay.cancelled || overlay.closed || overlay.settleId ||
                !focus || overlay.windows.has(focus) || spawnedBy(focus.get_pid(), overlay.pid))
                return GLib.SOURCE_REMOVE;
            const windows = [...overlay.windows.keys()];
            const monitor = global.display.get_current_monitor();
            const target = windows.find(w => w.get_monitor() === monitor) ?? windows[0];
            if (target) {
                this._log(`${focus.get_wm_class() ?? 'a window'} took the focus from the overlay, taking it back`);
                Main.activateWindow(target);
            }
            return GLib.SOURCE_REMOVE;
        });
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
        if (freeze.drag?.held)
            this._watchDrag();
        else
            freeze.actor.reactive = false;
        return true;
    }

    _breakGrab(win) {
        if (!this._armed)
            return;
        const {t0, timeoutId} = this._armed;
        removeSource(timeoutId);
        this._armed = null;
        this._trace(t0, 'overlay first frame');

        const windows = global.get_window_actors()
            .map(a => a.get_meta_window())
            .filter(w => w);
        const popups = windows
            .filter(w => POPUP_TYPES.includes(w.get_window_type()) &&
                w.get_client_type() === Meta.WindowClientType.WAYLAND &&
                !this._isOverlay(w))
            .map(w => w.get_frame_rect());
        if (popups.length === 0) {
            this._trace(t0, 'no wayland popup, no click');
            return;
        }

        const area = Main.layoutManager.getWorkAreaForMonitor(win.get_monitor());
        const target = pickClickPoint(area, popups);
        const rects = popups.map(r => `${r.x},${r.y} ${r.width}x${r.height}`).join('; ');
        if (!target) {
            this._trace(t0, `popups [${rects}] cover every click point, no click`);
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
            if (this._frozen?.drag?.held && actor) {
                // The freeze holds the pointer during a drag; retry on release.
                this._frozen.drag.afterRelease = () =>
                    this._clickWhenSettled(win, overlay, target, rects, t0, 0);
                return;
            }
            if (attempt >= SETTLE_ATTEMPTS || !actor) {
                const why = settled
                    ? `target ${target.x},${target.y} is not over the overlay`
                    : 'overlay never settled';
                this._trace(t0, `popups [${rects}], ${why}, no click`);
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
        const seat = defaultSeat(global.stage, Clutter);
        const ptr = seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
        ptr.notify_absolute_motion(Clutter.CURRENT_TIME, target.x, target.y);
        ptr.notify_button(Clutter.CURRENT_TIME, Clutter.BUTTON_MIDDLE, Clutter.ButtonState.PRESSED);
        ptr.notify_button(Clutter.CURRENT_TIME, Clutter.BUTTON_MIDDLE, Clutter.ButtonState.RELEASED);
        ptr.notify_absolute_motion(Clutter.CURRENT_TIME, x, y);
        this._trace(t0, `popups [${rects}], clicked ${target.x},${target.y} after ${attempt} waits, pointer back to ${x},${y}`);
    }
}
