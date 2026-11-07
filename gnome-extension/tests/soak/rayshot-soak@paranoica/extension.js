import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {MAX_FAILURES} from '../rayshot@paranoica/backoff.js';
import {defaultSeat} from '../rayshot@paranoica/compat.js';

const TICK_MS = 20;
const STEP_TIMEOUT_MS = 5000;

function rssKb(pid) {
    try {
        const [, bytes] = GLib.file_get_contents(`/proc/${pid}/status`);
        const m = /VmRSS:\s+(\d+)/.exec(new TextDecoder().decode(bytes));
        return m ? Number(m[1]) : null;
    } catch (_) {
        return null;
    }
}

function vramMib(pid) {
    try {
        const [ok, out] = GLib.spawn_command_line_sync(
            'nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader,nounits');
        if (!ok)
            return null;
        const line = new TextDecoder().decode(out).split('\n')
            .find(l => l.split(',')[0].trim() === `${pid}`);
        return line ? Number(line.split(',')[1]) : null;
    } catch (_) {
        return null;
    }
}

function serveProcesses() {
    const shell = `${new TextDecoder().decode(GLib.file_get_contents('/proc/self/stat')[1]).split(' ')[0]}`;
    let count = 0;
    // GLib.Dir is not usable from GJS on GNOME 45.
    const dir = Gio.File.new_for_path('/proc')
        .enumerate_children('standard::name', Gio.FileQueryInfoFlags.NONE, null);
    for (let info; (info = dir.next_file(null));) {
        const name = info.get_name();
        if (!/^\d+$/.test(name))
            continue;
        try {
            const stat = new TextDecoder().decode(GLib.file_get_contents(`/proc/${name}/stat`)[1]);
            const ppid = stat.slice(stat.lastIndexOf(')') + 2).split(' ')[1];
            const cmd = new TextDecoder().decode(GLib.file_get_contents(`/proc/${name}/cmdline`)[1]);
            if (ppid === shell && cmd.split('\0')[1] === 'serve')
                count++;
        } catch (_) {}
    }
    dir.close(null);
    return count;
}

// GNOME 45 keeps the body in bannerBodyText.
function noticeText(n) {
    return `${n.title} ${n.body ?? n.bannerBodyText ?? ''}`;
}

const DISPLAY_CONFIG = ['org.gnome.Mutter.DisplayConfig', '/org/gnome/Mutter/DisplayConfig'];

function displayConfig(iface, method, params, done, fail) {
    Gio.DBus.session.call(...DISPLAY_CONFIG, iface, method, params, null,
        Gio.DBusCallFlags.NONE, -1, null, (conn, res) => {
            let reply;
            try {
                reply = conn.call_finish(res);
            } catch (e) {
                fail(e.message);
                return;
            }
            done(reply);
        });
}

function waitFor(cond, then, fail, timeoutMs = STEP_TIMEOUT_MS) {
    const started = GLib.get_monotonic_time();
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, TICK_MS, () => {
        if (cond()) {
            then();
            return GLib.SOURCE_REMOVE;
        }
        if (GLib.get_monotonic_time() - started > timeoutMs * 1000) {
            fail();
            return GLib.SOURCE_REMOVE;
        }
        return GLib.SOURCE_CONTINUE;
    });
}

export default class SoakDriver extends Extension {
    enable() {
        // GNOME re-enables later extensions when an earlier one is disabled,
        // as on lock; the run must not start over.
        if (this._started)
            return;
        this._started = true;
        this._out = GLib.getenv('RAYSHOT_SOAK_OUT');
        this._warm = Number(GLib.getenv('RAYSHOT_SOAK_WARM') ?? '100');
        this._cold = Number(GLib.getenv('RAYSHOT_SOAK_COLD') ?? '20');
        this._samples = [];
        this._scenarioResults = [];
        this._driverCrashes = 0;
        this._lastSegv = 0;
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => {
            this._start();
            return GLib.SOURCE_REMOVE;
        });
    }

    disable() {}

    _start() {
        this._rayshot = Main.extensionManager.lookup('rayshot@paranoica')?.stateObj;
        if (!this._rayshot) {
            this._finish('rayshot extension not loaded');
            return;
        }
        // The NVIDIA driver now and then segfaults while a spare opens its
        // windows; such a session falls back to one-shot and is counted.
        const onServerExit = this._rayshot._onServerExit.bind(this._rayshot);
        this._rayshot._onServerExit = (server, epoch) => {
            if (server.proc.get_if_signaled() && server.proc.get_term_sig() === 11)
                this._lastSegv = GLib.get_monotonic_time();
            onServerExit(server, epoch);
        };
        const unfreeze = this._rayshot._unfreeze.bind(this._rayshot);
        this._rayshot._unfreeze = () => {
            if (this._sessionPid) {
                this._leftAtUnfreeze = global.get_window_actors()
                    .filter(a => a.get_meta_window()?.get_pid() === this._sessionPid).length;
            }
            unfreeze();
        };
        waitFor(() => this._rayshot._server?.ready, () => this._run('warm', this._warm, 0),
            () => this._finish('server never became ready'));
    }

    _sample(phase, i) {
        const server = this._rayshot._server;
        this._samples.push({
            phase,
            i,
            shell_rss_kb: rssKb('self'),
            server_pid: server?.pid ?? null,
            serve_processes: serveProcesses(),
            server_rss_kb: server ? rssKb(server.pid) : null,
            server_vram_mib: server ? vramMib(server.pid) : null,
        });
    }

    _run(phase, total, i) {
        const r = this._rayshot;
        if (phase === 'warm' && !r._server?.ready) {
            waitFor(() => r._server?.ready, () => this._run(phase, total, i),
                () => this._finish(`${phase} ${i}: no warm spare in time`));
            return;
        }
        // Cold runs are short, so they are sampled more often.
        if (i % (phase === 'cold' ? 5 : 10) === 0 || i === total)
            this._sample(phase, i);
        if (i === total) {
            if (phase === 'warm' && this._cold > 0) {
                this._rayshot._settings.set_boolean('warm-overlay', false);
                waitFor(() => !this._rayshot._server, () => this._run('cold', this._cold, 0),
                    () => this._finish('server did not stop'));
            } else {
                this._scenarios();
            }
            return;
        }
        r._capture();
        waitFor(() => r._overlay?.shown && r._overlay.framePath, () => {
            const o = r._overlay;
            if (phase === 'warm' && !o.server) {
                const crashed = GLib.get_monotonic_time() - this._lastSegv < 5 * GLib.USEC_PER_SEC;
                if (!crashed) {
                    this._finish(`warm ${i}: session took the one-shot path`);
                    return;
                }
                this._driverCrashes++;
                this._lastSegv = 0;
            }
            this._sessionPid = o.pid;
            this._leftAtUnfreeze = null;
            const line = o.server ? `cancel ${o.session}\n` : 'quit\n';
            try {
                o.proc.get_stdin_pipe().write_all(new TextEncoder().encode(line), null);
            } catch (e) {
                this._finish(`${phase} ${i}: cannot end the session: ${e.message}`);
                return;
            }
            waitFor(() => !r._busy, () => {
                const left = this._leftAtUnfreeze;
                this._sessionPid = null;
                if (left === null)
                    this._finish(`${phase} ${i}: unfreeze was not observed`);
                else if (left > 0)
                    this._finish(`${phase} ${i}: ${left} overlay window(s) still mapped at unfreeze`);
                else
                    this._run(phase, total, i + 1);
            }, () => this._finish(`${phase} ${i}: session did not end`));
        }, () => this._finish(`${phase} ${i}: overlay did not show`));
    }

    _scenarios() {
        const r = this._rayshot;
        r._settings.set_boolean('warm-overlay', true);
        // A virtual device drops events sent right after it is created.
        this._keyboard = defaultSeat(global.stage, Clutter)
            .create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        const steps = [
            next => this._copies(next),
            next => this._coldCopies(next),
            next => this._copyFails(next),
            next => this._coldCopyFails(next),
            next => this._copiesText(next),
            next => this._focusStolen(next),
            next => this._heldKey(next),
            next => this._spareCrashes(next),
            next => this._freshFrames(next),
            next => this._layoutChanges(next),
            next => this._screensSleep(next),
            next => this._lockUnlock(next),
            next => this._cancelBeforeFirstFrame(next, 0, {cancels: 0, framesAfter: 0, shown: 0}),
            next => this._oldAutostart(next),
            next => this._spareGivesUp(next),
            next => this._missingBinary(next),
        ];
        const run = i => {
            if (i === steps.length) {
                this._finish(null);
                return;
            }
            waitFor(() => !r._busy && r._server?.ready, () => steps[i](() => run(i + 1)),
                () => this._finish(`scenario ${i}: no warm spare in time`));
        };
        run(0);
    }

    _result(name, ok, detail) {
        this._scenarioResults.push({name, ok, detail});
    }

    _endSession(then) {
        const r = this._rayshot;
        const o = r._overlay;
        if (o) {
            try {
                const line = o.server ? `cancel ${o.session}\n` : 'quit\n';
                o.proc.get_stdin_pipe().write_all(new TextEncoder().encode(line), null);
            } catch (e) {
                this._finish(`cannot end the session: ${e.message}`);
                return;
            }
        }
        waitFor(() => !r._busy, then, () => this._finish('session did not end'));
    }

    // Enter in the overlay copies through wl-copy, which needs the focus for a
    // moment on compositors without a data-control protocol.
    _coldCopies(next) {
        const r = this._rayshot;
        r._settings.set_boolean('warm-overlay', false);
        waitFor(() => !r._server && !r._respawnId, () => this._copies(() => {
            r._settings.set_boolean('warm-overlay', true);
            next();
        }, 'enter copies the selection without a spare', true),
        () => this._finish('warm off did not stop the spare'));
    }

    // wl-copy that never finishes must not leave the freeze up, and the user
    // has to learn that the clipboard is empty.
    _coldCopyFails(next) {
        const r = this._rayshot;
        r._settings.set_boolean('warm-overlay', false);
        waitFor(() => !r._server && !r._respawnId, () => this._copyFails(() => {
            r._settings.set_boolean('warm-overlay', true);
            next();
        }, 'copy that never finishes without a spare'),
        () => this._finish('warm off did not stop the spare'));
    }

    _copyFails(next, name = 'copy that never finishes') {
        const r = this._rayshot;
        const before = new Set(Main.messageTray.getSources().flatMap(src => src.notifications ?? []));
        const flag = GLib.getenv('RAYSHOT_SOAK_WLCOPY_HANG');
        GLib.file_set_contents(flag, '');
        const disarm = () => GLib.unlink(flag);
        const notified = () => Main.messageTray.getSources()
            .flatMap(source => source.notifications ?? [])
            .find(n => !before.has(n) && /clipboard/i.test(noticeText(n)));
        r._capture();
        waitFor(() => r._overlay?.shown && r._overlay.framePath, () => {
            const o = r._overlay;
            try {
                o.proc.get_stdin_pipe().write_all(
                    new TextEncoder().encode(`select ${o.session} 100 100 600 400\n`), null);
            } catch (e) {
                disarm();
                this._finish(`${name}: cannot select: ${e.message}`);
                return;
            }
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
                const now = GLib.get_monotonic_time();
                this._keyboard.notify_keyval(now, Clutter.KEY_Return, Clutter.KeyState.PRESSED);
                this._keyboard.notify_keyval(now + 1000, Clutter.KEY_Return, Clutter.KeyState.RELEASED);
                waitFor(() => !r._busy && notified(), () => {
                    disarm();
                    const n = notified();
                    const notices = Main.messageTray.getSources()
                        .flatMap(source => source.notifications ?? [])
                        .filter(m => /clipboard/i.test(noticeText(m))).length;
                    let problem = null;
                    if (n.isTransient)
                        problem = 'notified, but the notice leaves the tray after its banner';
                    else if (notices !== 1)
                        problem = `${notices} clipboard notices in the tray, expected 1`;
                    this._result(name, !problem, problem ?? `overlay closed, notified: "${noticeText(n)}"`);
                    next();
                }, () => {
                    disarm();
                    const tray = Main.messageTray.getSources()
                        .map(src => `${src.title}: ${(src.notifications ?? []).map(n => n.title).join(' | ')}`);
                    this._result(name, false, (r._busy ? 'overlay still up'
                        : 'overlay closed without a notification') + `; tray: ${tray.join('; ') || 'empty'}`);
                    if (r._busy)
                        this._endSession(next);
                    else
                        next();
                }, 8000);
                return GLib.SOURCE_REMOVE;
            });
        }, () => {
            disarm();
            this._finish(`${name}: overlay did not show`);
        });
    }

    _copiesText(next) {
        const r = this._rayshot;
        const name = 'ctrl shift c copies the text';
        const tesseract = GLib.find_program_in_path('tesseract') !== null;
        const scratch = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'rayshot']);
        const pngs = () => {
            const [, out] = GLib.spawn_command_line_sync(`find ${GLib.shell_quote(scratch)} -maxdepth 1 -name '*.png'`);
            return new TextDecoder().decode(out).split('\n').filter(Boolean);
        };
        const kept = new Set(pngs());
        const label = new St.Label({
            text: 'RAYSHOT 4217', x: 120, y: 120,
            style: 'background-color: #ffffff; color: #000000; font-size: 40px; padding: 20px;',
        });
        Main.layoutManager.uiGroup.add_child(label);
        const clipboard = St.Clipboard.get_default();
        clipboard.set_text(St.ClipboardType.CLIPBOARD, 'rayshot-soak-marker');
        const before = new Set(Main.messageTray.getSources().flatMap(src => src.notifications ?? []));
        const notice = () => Main.messageTray.getSources()
            .flatMap(src => src.notifications ?? [])
            .find(n => !before.has(n) && /tesseract/i.test(noticeText(n)));
        let copied = '';
        const poll = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 200, () => {
            clipboard.get_text(St.ClipboardType.CLIPBOARD, (_c, text) => {
                copied = text ?? '';
            });
            return GLib.SOURCE_CONTINUE;
        });
        const cleanup = () => {
            GLib.source_remove(poll);
            label.destroy();
        };
        const done = (ok, detail) => {
            cleanup();
            const left = pngs().filter(f => !kept.has(f));
            if (ok && left.length)
                this._result(name, false, `${detail}; left behind: ${left.join(', ')}`);
            else
                this._result(name, ok, detail);
            next();
        };
        const press = () => {
            const k = this._keyboard;
            const t = GLib.get_monotonic_time();
            const keys = [Clutter.KEY_Control_L, Clutter.KEY_Shift_L, Clutter.KEY_c];
            keys.forEach((key, i) => k.notify_keyval(t + i * 1000, key, Clutter.KeyState.PRESSED));
            [...keys].reverse().forEach((key, i) =>
                k.notify_keyval(t + (keys.length + i) * 1000, key, Clutter.KeyState.RELEASED));
        };
        const id = global.stage.connect('after-paint', () => {
            global.stage.disconnect(id);
            r._capture();
            waitFor(() => r._overlay?.shown && r._overlay.framePath, () => {
                const o = r._overlay;
                try {
                    o.proc.get_stdin_pipe().write_all(
                        new TextEncoder().encode(`select ${o.session} 100 100 600 300\n`), null);
                } catch (e) {
                    cleanup();
                    this._finish(`${name}: cannot select: ${e.message}`);
                    return;
                }
                GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
                    press();
                    waitFor(() => !r._busy, () => {
                        if (tesseract) {
                            waitFor(() => copied.includes('4217'),
                                () => done(true, `clipboard holds "${copied.trim()}"`),
                                () => done(false, `clipboard holds "${copied}"`), 20000);
                        } else {
                            waitFor(() => notice(),
                                () => done(true, `no tesseract, notified: "${noticeText(notice())}"`),
                                () => done(false, 'no tesseract and no notice'), 20000);
                        }
                    }, () => {
                        cleanup();
                        this._result(name, false, 'overlay did not close after ctrl shift c');
                        this._endSession(next);
                    });
                    return GLib.SOURCE_REMOVE;
                });
            }, () => {
                cleanup();
                this._finish(`${name}: overlay did not show`);
            });
        });
        global.stage.queue_redraw();
    }

    _copies(next, name = 'enter copies the selection', cold = false) {
        const r = this._rayshot;
        const clipboard = St.Clipboard.get_default();
        clipboard.set_text(St.ClipboardType.CLIPBOARD, 'rayshot-soak-marker');
        r._capture();
        waitFor(() => r._overlay?.shown && r._overlay.framePath, () => {
            const o = r._overlay;
            if (Boolean(o.server) === cold) {
                this._finish(`${name}: wrong overlay path`);
                return;
            }
            try {
                o.proc.get_stdin_pipe().write_all(
                    new TextEncoder().encode(`select ${o.session} 100 100 600 400\n`), null);
            } catch (e) {
                this._finish(`${name}: cannot select: ${e.message}`);
                return;
            }
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
                const keyboard = this._keyboard;
                const now = GLib.get_monotonic_time();
                keyboard.notify_keyval(now, Clutter.KEY_Return, Clutter.KeyState.PRESSED);
                keyboard.notify_keyval(now + 1000, Clutter.KEY_Return, Clutter.KeyState.RELEASED);
                waitFor(() => !r._busy, () => {
                    GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
                        const types = clipboard.get_mimetypes(St.ClipboardType.CLIPBOARD);
                        const ok = types.includes('image/png');
                        this._result(name, ok, ok ? 'clipboard holds the png'
                            : `clipboard holds ${types.join(', ') || 'nothing'}`);
                        next();
                        return GLib.SOURCE_REMOVE;
                    });
                }, () => {
                    this._result(name, false, 'overlay did not close after enter');
                    this._endSession(next);
                });
                return GLib.SOURCE_REMOVE;
            });
        }, () => this._finish(`${name}: overlay did not show`));
    }

    // A spare that dies while opening its windows, as the NVIDIA driver does
    // now and then, must be replaced before the freeze feels stuck.
    _spareCrashes(next) {
        const r = this._rayshot;
        const name = 'spare crashes while opening its windows';
        let killedAt = 0;
        const createdId = global.display.connect('window-created', (_d, win) => {
            const server = r._overlay?.server;
            if (killedAt || !server || !r._isOverlay(win))
                return;
            killedAt = GLib.get_monotonic_time();
            // SIGABRT dumps core like a real crash; a sent SIGSEGV only resets
            // Rust's stack-overflow handler and the process lives on.
            server.proc.send_signal(6);
        });
        r._capture();
        waitFor(() => killedAt && r._overlay && !r._overlay.server, () => {
            global.display.disconnect(createdId);
            const ms = Math.round((GLib.get_monotonic_time() - killedAt) / 1000);
            this._result(name, ms < 300, `one-shot overlay ${ms} ms after the crash (limit 300)`);
            waitFor(() => r._overlay?.shown && r._overlay.framePath, () => this._endSession(next),
                () => this._finish(`${name}: one-shot overlay did not show`));
        }, () => {
            global.display.disconnect(createdId);
            this._finish(`${name}: ${killedAt ? 'no one-shot after the crash' : 'no overlay window to crash'}`);
        });
    }

    // Stands in for a stray click that focuses and raises an app hidden under
    // the freeze.
    _focusStolen(next) {
        const r = this._rayshot;
        const name = 'app takes the focus from an open overlay';
        const helperWindow = () => global.get_window_actors().map(a => a.get_meta_window())
            .find(w => w?.get_title() === 'rayshot-soak-helper');
        try {
            const launcher = new Gio.SubprocessLauncher();
            launcher.setenv('RAYSHOT_SOAK_HELPER_LOG', this._helperLog(), true);
            this._helper = launcher.spawnv(['gjs', '-m', `${this.path}/helper-window.js`]);
        } catch (e) {
            this._finish(`cannot start the helper window: ${e.message}`);
            return;
        }
        // A window activated before it is ready to take the focus is passed
        // over, so the helper has to hold it once before the capture.
        waitFor(() => {
            const helper = helperWindow();
            if (!helper?.get_compositor_private())
                return false;
            if (global.display.focus_window !== helper)
                Main.activateWindow(helper, global.display.get_current_time_roundtrip());
            return global.display.focus_window === helper;
        }, () => {
            const helper = helperWindow();
            r._capture();
            waitFor(() => r._overlay?.shown && r._overlay.framePath, () => {
                const overlays = [...r._overlay.windows.keys()];
                // The guard may take the focus back before this code runs again,
                // so the helper's turn is caught as it happens.
                let helperFocused = false;
                const focusId = global.display.connect('notify::focus-window', () => {
                    if (global.display.focus_window === helper)
                        helperFocused = true;
                });
                const done = (ok, detail) => {
                    global.display.disconnect(focusId);
                    this._result(name, ok, detail);
                    this._endSession(next);
                };
                Main.activateWindow(helper, global.display.get_current_time_roundtrip());
                waitFor(() => helperFocused && overlays.includes(global.display.focus_window), () => {
                    const top = global.display.sort_windows_by_stacking([helper, ...overlays]).at(-1);
                    const onTop = overlays.includes(top);
                    done(onTop, `focus came back, overlay ${onTop ? 'on top' : 'under the app'}`);
                }, () => done(false, helperFocused
                    ? `focus stayed on ${global.display.focus_window?.get_title()}`
                    : 'the app never got the focus, nothing was tested'), 1000);
            }, () => this._finish(`${name}: overlay did not show`));
        }, () => this._finish(`${name}: helper window did not show`));
    }

    // Esc on the freeze races the overlay's first frame; a cancelled capture
    // must not show, focus or click anything when that frame arrives.
    _cancelBeforeFirstFrame(next, i, stats) {
        const r = this._rayshot;
        const name = 'first frame after cancelling';
        if (i === 10) {
            const ok = stats.shown === 0 && stats.framesAfter > 0;
            this._result(name, ok, `${stats.cancels} cancels, ${stats.framesAfter} first frames after the ` +
                `cancel, ${stats.shown} shown over the freeze` +
                (stats.framesAfter ? '' : ' (race never hit, scenario is blind)'));
            next();
            return;
        }
        const show = r._showThroughFreeze;
        r._showThroughFreeze = actor => {
            if (r._overlay?.cancelled)
                stats.shown++;
            return show.call(r, actor);
        };
        const createdId = global.display.connect('window-created', (_d, win) => {
            if (!r._isOverlay(win) || !r._overlay)
                return;
            if (!r._overlay.cancelled) {
                r._cancelCapture();
                stats.cancels++;
            }
            const actor = win.get_compositor_private();
            const id = actor?.connect('first-frame', () => {
                actor.disconnect(id);
                if (r._overlay?.cancelled)
                    stats.framesAfter++;
            });
        });
        r._capture();
        waitFor(() => !r._busy, () => {
            global.display.disconnect(createdId);
            r._showThroughFreeze = show;
            waitFor(() => r._server?.ready, () => this._cancelBeforeFirstFrame(next, i + 1, stats),
                () => this._finish(`${name}: no warm spare in time`));
        }, () => this._finish(`${name}: session did not end`));
    }

    // Users of the old daemon still have its autostart entry, which runs
    // `rayshot daemon` at login; that is the one chance to tell them to finish
    // the update.
    _oldAutostart(next) {
        const name = 'old daemon autostart at login';
        const home = GLib.build_filenamev([GLib.path_get_dirname(this._out), 'old-home']);
        const entry = GLib.build_filenamev([home, '.config', 'autostart', 'rayshot-daemon.desktop']);
        GLib.mkdir_with_parents(GLib.path_get_dirname(entry), 0o700);
        GLib.file_set_contents(entry, '[Desktop Entry]\nExec=rayshot daemon\n');
        const before = new Set(Main.messageTray.getSources().flatMap(src => src.notifications ?? []));
        const notice = () => Main.messageTray.getSources().flatMap(src => src.notifications ?? [])
            .find(n => !before.has(n) && /install-hotkey/.test(noticeText(n)));
        const launcher = new Gio.SubprocessLauncher();
        launcher.setenv('HOME', home, true);
        let proc;
        try {
            proc = launcher.spawnv([this._rayshot._bin(), 'daemon']);
        } catch (e) {
            this._finish(`${name}: ${e.message}`);
            return;
        }
        proc.wait_async(null, () => {
            waitFor(notice, () => {
                const gone = !GLib.file_test(entry, GLib.FileTest.EXISTS);
                this._result(name, gone, gone ? `notified: "${noticeText(notice())}"`
                    : 'notified, but the autostart entry is still there');
                next();
            }, () => {
                this._result(name, false, 'no notification about finishing the update');
                next();
            }, 3000);
        });
    }

    _helperLog() {
        return GLib.build_filenamev([GLib.path_get_dirname(this._out), 'helper-keys.log']);
    }

    // A key held down when Print is pressed has its release land on the
    // freeze. If the capture is cancelled, the app must still learn the key
    // is up, or it keeps repeating it.
    _heldKey(next) {
        const r = this._rayshot;
        const name = 'key held through a cancelled capture';
        const helper = global.get_window_actors().map(a => a.get_meta_window())
            .find(w => w?.get_title() === 'rayshot-soak-helper');
        if (!helper) {
            this._finish(`${name}: no helper window`);
            return;
        }
        const log = this._helperLog();
        const read = () => {
            try {
                return new TextDecoder().decode(GLib.file_get_contents(log)[1]).trim().split('\n');
            } catch (_) {
                return [];
            }
        };
        const key = (state, then) => {
            this._keyboard.notify_keyval(GLib.get_monotonic_time(), Clutter.KEY_a, state);
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
                then();
                return GLib.SOURCE_REMOVE;
            });
        };
        Main.activateWindow(helper, global.display.get_current_time_roundtrip());
        waitFor(() => global.display.focus_window === helper, () => {
            GLib.file_set_contents(log, '');
            key(Clutter.KeyState.PRESSED, () => {
                if (!read().includes(`press ${Clutter.KEY_a}`)) {
                    this._result(name, false, 'the app never saw the key go down, nothing was tested');
                    key(Clutter.KeyState.RELEASED, next);
                    return;
                }
                r._capture();
                let atCancel = 0;
                waitFor(() => r._frozen, () => key(Clutter.KeyState.RELEASED, () => {
                    atCancel = read().length;
                    r._cancelCapture();
                    // Long enough for the app's own key repeat to show a stuck key.
                    waitFor(() => !r._busy, () => GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1500, () => {
                        const after = read();
                        const repeats = after.slice(atCancel).filter(l => l === `press ${Clutter.KEY_a}`).length;
                        const up = (after.includes(`release ${Clutter.KEY_a}`) || after.includes('focus out')) &&
                            repeats === 0;
                        this._result(name, up, up ? `the app saw the key go up (${after.join(', ')})`
                            : `the key is still down for the app (${after.join(', ')})`);
                        next();
                        return GLib.SOURCE_REMOVE;
                    }), () => this._finish(`${name}: capture did not end`));
                }), () => this._finish(`${name}: no freeze`));
            });
        }, () => this._finish(`${name}: the helper never got the focus`));
    }

    // One capture, then `check(overlay)` while it is shown; `then` once it ended.
    _captureAndCheck(name, check, then) {
        const r = this._rayshot;
        r._capture();
        waitFor(() => r._overlay?.shown && r._overlay.framePath, () => {
            const problem = check(r._overlay);
            this._endSession(() => then(problem));
        }, () => this._finish(`${name}: overlay did not show`));
    }

    // The old daemon could hand out a picture from before the screens slept;
    // every capture must show what is on screen right now.
    _freshFrames(next) {
        const r = this._rayshot;
        const name = 'each capture shows the screen as it is now';
        const square = new St.Widget({x: 60, y: 60, width: 40, height: 40, style: 'background-color: #00ff00;'});
        Main.layoutManager.uiGroup.add_child(square);
        const greens = [];
        const writeFrame = r._writeFrame;
        r._writeFrame = (pixbuf, t0) => {
            greens.push(pixbuf.read_pixel_bytes().get_data()[80 * pixbuf.get_rowstride() + 80 * 4 + 1]);
            return writeFrame.call(r, pixbuf, t0);
        };
        const done = (ok, detail) => {
            r._writeFrame = writeFrame;
            square.destroy();
            this._result(name, ok, detail);
            next();
        };
        // A capture takes the last painted frame, so the square must be drawn first.
        const painted = then => {
            const id = global.stage.connect('after-paint', () => {
                global.stage.disconnect(id);
                then();
            });
            global.stage.queue_redraw();
        };
        painted(() => this._captureAndCheck(name, () => null, () => {
            square.style = 'background-color: #000000;';
            waitFor(() => r._server?.ready, () => painted(() => this._captureAndCheck(name, () => null, () => {
                const [first, second] = greens;
                done(first > 200 && second < 60, `green ${first} then ${second} after the square turned black`);
            })), () => this._finish(`${name}: no warm spare in time`));
        }));
    }

    // A monitor that is unplugged or switched off while the spare waits must
    // not leave the spare drawing on a layout that no longer exists.
    _layoutChanges(next) {
        const r = this._rayshot;
        const name = 'monitors change while the spare waits';
        const fail = why => this._finish(`${name}: ${why}`);
        const apply = (serial, logical, then) => displayConfig('org.gnome.Mutter.DisplayConfig', 'ApplyMonitorsConfig',
            new GLib.Variant('(uua(iiduba(ssa{sv}))a{sv})', [serial, 1, logical, {}]), then, fail);
        const state = then => displayConfig('org.gnome.Mutter.DisplayConfig', 'GetCurrentState', null,
            reply => then(reply.recursiveUnpack()), fail);
        const shape = overlay => {
            const monitors = [...overlay.windows.keys()].map(w => w.get_monitor()).sort().join(',');
            return `${overlay.windows.size} windows on monitors ${monitors || 'none'}, frame ${overlay.texture.get_width()}`;
        };
        state(([serial, monitors, logical]) => {
            const mode = Object.fromEntries(monitors.map(([spec, modes]) =>
                [spec[0], modes.find(m => m[6]['is-current'])?.[0]]));
            if (Object.values(mode).some(m => !m)) {
                fail('a monitor has no current mode');
                return;
            }
            const layout = logical.map(([x, y, scale, transform, primary, specs]) =>
                [x, y, scale, transform, primary, specs.map(spec => [spec[0], mode[spec[0]], {}])]);
            const primary = layout.find(l => l[4]);
            if (layout.length < 2 || !primary) {
                fail(`needs two monitors, got ${layout.length}`);
                return;
            }
            apply(serial, [[0, 0, primary[2], primary[3], true, primary[5]]], () => {
                waitFor(() => global.display.get_n_monitors() === 1 && r._server?.ready, () => {
                    this._captureAndCheck(name, shape, one => {
                        state(([serial2]) => apply(serial2, layout, () => {
                            waitFor(() => global.display.get_n_monitors() === 2 && r._server?.ready, () => {
                                this._captureAndCheck(name, shape, two => {
                                    const ok = one === '1 windows on monitors 0, frame 1920' &&
                                        two === '2 windows on monitors 0,1, frame 3840';
                                    this._result(name, ok, `one monitor: ${one}; both again: ${two}`);
                                    next();
                                });
                            }, () => fail('the second monitor did not come back'));
                        }));
                    });
                }, () => fail('the second monitor did not go away'));
            });
        });
    }

    _screensSleep(next) {
        const r = this._rayshot;
        const name = 'capture after the screens slept';
        const fail = why => this._finish(`${name}: ${why}`);
        const powerSave = (mode, then) => displayConfig('org.freedesktop.DBus.Properties', 'Set',
            new GLib.Variant('(ssv)', ['org.gnome.Mutter.DisplayConfig', 'PowerSaveMode', new GLib.Variant('i', mode)]),
            then, fail);
        powerSave(3, () => GLib.timeout_add(GLib.PRIORITY_DEFAULT, 3000, () => {
            powerSave(0, () => waitFor(() => r._server?.ready, () => {
                this._captureAndCheck(name, overlay => overlay.server ? null : 'took the one-shot path', problem => {
                    this._result(name, !problem, problem ?? 'warm capture after 3 s of power save');
                    next();
                });
            }, () => fail('no warm spare after waking')));
            return GLib.SOURCE_REMOVE;
        }));
    }

    // GNOME disables extensions on lock, which also happens on suspend, and
    // enables them again on unlock.
    _lockUnlock(next) {
        const r = this._rayshot;
        const name = 'capture after lock and unlock';
        const fail = why => this._finish(`${name}: ${why}`);
        // A few failures short of giving up must not follow the user past a lock.
        r._serverFailures = MAX_FAILURES - 1;
        Main.sessionMode.pushMode('unlock-dialog');
        waitFor(() => !r._settings, () => {
            Main.sessionMode.popMode('unlock-dialog');
            waitFor(() => r._settings && r._server?.ready, () => {
                const spares = serveProcesses();
                const failures = r._serverFailures;
                this._captureAndCheck(name, overlay => overlay.server ? null : 'took the one-shot path', problem => {
                    const ok = !problem && spares === 1 && failures === 0;
                    this._result(name, ok, problem ??
                        `${spares} spare(s) after unlock, ${failures} failures carried over, warm capture`);
                    next();
                });
            }, () => fail('the extension did not come back after unlock'));
        }, () => fail('locking did not disable the extension'));
    }

    _restartWarm(then) {
        const r = this._rayshot;
        r._settings.set_boolean('warm-overlay', false);
        waitFor(() => !r._server && !r._respawnId, () => {
            r._settings.set_boolean('warm-overlay', true);
            then();
        }, () => this._finish('warm off did not stop the spare'));
    }

    _spareGivesUp(next) {
        const r = this._rayshot;
        const name = 'spare that keeps failing';
        const bin = r._bin;
        r._bin = () => '/usr/bin/false';
        this._restartWarm(() => this._watchGiveUp(next, name, bin));
    }

    // Lock and unlock re-enable the extension; a spare that gave up must stay
    // given up and must not notify again. Calls back with a problem or null.
    _throughLock(stopped, failures, then) {
        const r = this._rayshot;
        const notices = () => Main.messageTray.getSources().flatMap(src => src.notifications ?? [])
            .filter(n => /keeps failing/.test(noticeText(n))).length;
        const before = notices();
        Main.sessionMode.pushMode('unlock-dialog');
        waitFor(() => !r._settings, () => {
            Main.sessionMode.popMode('unlock-dialog');
            waitFor(() => r._settings, () => GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1500, () => {
                if (!stopped() || r._serverFailures < failures)
                    then('restarted after unlock');
                else if (notices() > before)
                    then('notified again after unlock');
                else
                    then(null);
                return GLib.SOURCE_REMOVE;
            }), () => then('the extension did not come back after unlock'));
        }, () => then('locking did not disable the extension'));
    }

    _watchGiveUp(next, name, bin) {
        const r = this._rayshot;
        const stopped = () => !r._server && !r._respawnId;
        waitFor(() => r._serverFailures >= MAX_FAILURES && stopped(), () => {
            const failures = r._serverFailures;
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1500, () => {
                const stayed = stopped() && r._serverFailures === failures;
                const told = r._notified.has('spare-failed');
                this._throughLock(stopped, failures, afterLock => {
                r._bin = bin;
                this._restartWarm(() => waitFor(() => r._server?.ready, () => {
                    this._result(name, stayed && told && !afterLock, `stopped after ${failures} failures, ` +
                        `${stayed ? 'stayed stopped' : 'restarted again'}, ` +
                        `${told ? 'notified' : 'no notification'}, ` +
                        `${afterLock ?? 'still stopped after lock and unlock'}, recovers after warm off/on`);
                    next();
                }, () => {
                    this._result(name, false, 'warm off/on did not bring the spare back');
                    next();
                }));
                });
                return GLib.SOURCE_REMOVE;
            });
        }, () => {
            r._bin = bin;
            this._result(name, false, `still restarting after ${r._serverFailures} failures`);
            this._restartWarm(() => waitFor(() => r._server?.ready, next,
                () => this._finish(`${name}: spare never came back`)));
        }, 15000);
    }

    // A binary installed after login must still get a spare, and the misses
    // before it must not count toward giving up on a later failure.
    _missingBinary(next) {
        const r = this._rayshot;
        const name = 'binary installed after login';
        const bin = r._bin;
        const fail = detail => {
            r._bin = bin;
            this._result(name, false, detail);
            this._restartWarm(() => waitFor(() => r._server?.ready, next,
                () => this._finish(`${name}: spare never came back`)));
        };
        r._bin = () => null;
        this._restartWarm(() => waitFor(() => r._binMisses > MAX_FAILURES, () => {
            if (!r._respawnId) {
                fail(`gave up looking after ${r._binMisses} misses`);
                return;
            }
            r._bin = bin;
            waitFor(() => r._server?.ready, () => {
                r._server.proc.force_exit();
                waitFor(() => r._server?.ready, () => {
                    const told = r._notified.has('spare-failed');
                    this._result(name, !told, told ? 'gave up after one failure'
                        : 'found on its own, a later failure is retried');
                    next();
                }, () => fail('a failure after the install was not retried'), 20000);
            }, () => fail('the installed binary was never picked up'), 20000);
        }, () => fail('misses never piled up'), 20000));
    }

    _finish(error) {
        this._rayshot?._settings?.set_boolean('warm-overlay', true);
        this._helper?.force_exit();
        const result = JSON.stringify({
            error,
            samples: this._samples,
            scenarios: this._scenarioResults,
            driverCrashes: this._driverCrashes,
        }, null, 1);
        if (this._out)
            GLib.file_set_contents(this._out, result);
        console.log(`rayshot-soak: done ${error ?? 'ok'}`);
    }
}
