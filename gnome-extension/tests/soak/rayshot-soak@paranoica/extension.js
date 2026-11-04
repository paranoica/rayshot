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
        if (i % 10 === 0 || i === total)
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
            next => this._focusStolen(next),
            next => this._spareCrashes(next),
            next => this._cancelBeforeFirstFrame(next, 0, {cancels: 0, framesAfter: 0, shown: 0}),
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
            this._helper = Gio.Subprocess.new(['gjs', '-m', `${this.path}/helper-window.js`],
                Gio.SubprocessFlags.NONE);
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

    _watchGiveUp(next, name, bin) {
        const r = this._rayshot;
        const stopped = () => !r._server && !r._respawnId;
        waitFor(() => r._serverFailures >= MAX_FAILURES && stopped(), () => {
            const failures = r._serverFailures;
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1500, () => {
                const stayed = stopped() && r._serverFailures === failures;
                const told = r._notified.has('spare-failed');
                r._bin = bin;
                this._restartWarm(() => waitFor(() => r._server?.ready, () => {
                    this._result(name, stayed && told, `stopped after ${failures} failures, ` +
                        `${stayed ? 'stayed stopped' : 'restarted again'}, ` +
                        `${told ? 'notified' : 'no notification'}, recovers after warm off/on`);
                    next();
                }, () => {
                    this._result(name, false, 'warm off/on did not bring the spare back');
                    next();
                }));
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
