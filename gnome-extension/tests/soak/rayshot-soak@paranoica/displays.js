import Clutter from 'gi://Clutter';
import GdkPixbuf from 'gi://GdkPixbuf';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {getInputSourceManager} from 'resource:///org/gnome/shell/ui/status/keyboard.js';

import {defaultSeat} from '../rayshot@paranoica/compat.js';

const TICK_MS = 20;
const STEP_MS = 30;
const MARK = 40;
const PAD = 40;
const EDGE = 3;
const COLOURS = [[255, 0, 0], [0, 255, 0], [0, 0, 255]];
const KEY_LEFTCTRL = 29;
const KEY_LEFTSHIFT = 42;
const KEY_C = 46;

const WANTS = {
    plain: [1, 1],
    scale2: [2, 2],
    fractional: [1.5, 1.5],
    mixed: [2, 1],
    left: [1, 1],
    vertical: [1, 1],
    portrait: [1, 1],
};

function waitFor(cond, then, fail, timeoutMs = 8000) {
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

function later(ms, then) {
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, ms, () => {
        then();
        return GLib.SOURCE_REMOVE;
    });
}

function sequence(steps, done) {
    const run = i => {
        if (i === steps.length) {
            done();
            return;
        }
        steps[i]();
        later(STEP_MS, () => run(i + 1));
    };
    run(0);
}

function painted(then) {
    const id = global.stage.connect('after-paint', () => {
        global.stage.disconnect(id);
        then();
    });
    global.stage.queue_redraw();
}

function monitors() {
    const n = global.display.get_n_monitors();
    return [...Array(n).keys()].map(i => global.display.get_monitor_geometry(i));
}

function call(method, params, done, fail) {
    Gio.DBus.session.call('org.gnome.Mutter.DisplayConfig', '/org/gnome/Mutter/DisplayConfig',
        'org.gnome.Mutter.DisplayConfig', method, params, null, Gio.DBusCallFlags.NONE, -1, null,
        (conn, res) => {
            try {
                done(conn.call_finish(res));
            } catch (e) {
                fail(e.message);
            }
        });
}

function plan(kind, state) {
    const [, outputs, , props] = state;
    const logical = (props['layout-mode'] ?? 1) === 1;
    const wants = WANTS[kind] ?? [1, 1];
    const placed = outputs.map(([spec, modes], i) => {
        const mode = modes.find(m => m[6]['is-current']) ?? modes[0];
        const want = wants[i] ?? 1;
        const scales = mode[5].length ? mode[5] : [1];
        const scale = scales.reduce((a, b) => Math.abs(b - want) < Math.abs(a - want) ? b : a);
        const transform = kind === 'portrait' && i === 1 ? 1 : 0;
        let [w, h] = transform % 2 ? [mode[2], mode[1]] : [mode[1], mode[2]];
        if (logical) {
            w = Math.round(w / scale);
            h = Math.round(h / scale);
        }
        return {connector: spec[0], mode: mode[0], scale, transform, w, h, x: 0, y: 0, primary: i === 0};
    });
    if (kind === 'left' && placed.length > 1) {
        placed[1].x = 0;
        placed[0].x = placed[1].w;
    } else if (kind === 'vertical') {
        placed.forEach((m, i) => {
            m.y = placed.slice(0, i).reduce((sum, p) => sum + p.h, 0);
        });
    } else {
        placed.forEach((m, i) => {
            m.x = placed.slice(0, i).reduce((sum, p) => sum + p.w, 0);
        });
    }
    return placed;
}

function matches(geoms, placed) {
    return geoms.length === placed.length && placed.every(p => geoms.some(g =>
        g.x === p.x && g.y === p.y && g.width === p.w && g.height === p.h));
}

function readPng(then, fail) {
    St.Clipboard.get_default().get_content(St.ClipboardType.CLIPBOARD, 'image/png', (_c, bytes) => {
        if (!bytes || bytes.get_size() === 0) {
            fail('the clipboard holds no png');
            return;
        }
        try {
            then(GdkPixbuf.Pixbuf.new_from_stream(Gio.MemoryInputStream.new_from_bytes(bytes), null));
        } catch (e) {
            fail(`the clipboard png does not decode: ${e.message}`);
        }
    });
}

function pixel(pb, x, y) {
    const ix = Math.round(x);
    const iy = Math.round(y);
    if (ix < 0 || iy < 0 || ix >= pb.get_width() || iy >= pb.get_height())
        return null;
    const at = iy * pb.get_rowstride() + ix * pb.get_n_channels();
    const data = pb.get_pixels();
    return [data[at], data[at + 1], data[at + 2]];
}

function near(a, b) {
    return a !== null && a.every((v, i) => Math.abs(v - b[i]) <= 40);
}

function hex(rgb) {
    return rgb ? `#${rgb.map(v => v.toString(16).padStart(2, '0')).join('')}` : 'outside';
}

export class Displays {
    constructor(driver) {
        this._d = driver;
        this._pointer = null;
        this._marks = [];
    }

    steps() {
        this._device();
        const cases = (GLib.getenv('RAYSHOT_SOAK_CASES') ?? 'plain').split(',').filter(Boolean);
        return cases.map(kind => next => {
            try {
                if (kind === 'russian')
                    this._russian(next);
                else if (kind === 'fullscreen')
                    this._fullscreen(next);
                else if (kind === 'shape-enter')
                    this._shapeEnter(next);
                else if (kind === 'text-enter' || kind === 'text-escape')
                    this._textKey(kind === 'text-enter' ? Clutter.KEY_Return : Clutter.KEY_Escape, next);
                else if (kind === 'spam')
                    this._spam(next);
                else
                    this._layoutCase(kind, next);
            } catch (e) {
                this._fail(kind, `the scenario threw: ${e.message}`, next);
            }
        });
    }

    _r() {
        return this._d._rayshot;
    }

    _device() {
        this._pointer ??= defaultSeat(global.stage, Clutter)
            .create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
        return this._pointer;
    }

    _fail(name, why, next) {
        this._clearMarks();
        this._d._result(name, false, why);
        if (this._r()._busy)
            this._d._endSession(next);
        else
            next();
    }

    _clearMarks() {
        this._marks.forEach(m => m.destroy());
        this._marks = [];
    }

    _mark(x, y, rgb) {
        const actor = new St.Widget({
            x, y, width: MARK, height: MARK,
            style: `background-color: rgb(${rgb.join(',')});`,
        });
        Main.layoutManager.uiGroup.add_child(actor);
        this._marks.push(actor);
    }

    _apply(kind, then, fail) {
        call('GetCurrentState', null, reply => {
            const state = reply.recursiveUnpack();
            const placed = plan(kind, state);
            const config = placed.map(p => [p.x, p.y, p.scale, p.transform, p.primary,
                [[p.connector, p.mode, {}]]]);
            call('ApplyMonitorsConfig', new GLib.Variant('(uua(iiduba(ssa{sv}))a{sv})',
                [state[0], 1, config, {}]), () => {
                waitFor(() => matches(monitors(), placed), () => later(300, () => then(placed)),
                    () => fail(`layout ${JSON.stringify(placed.map(p => [p.x, p.y, p.w, p.h, p.scale]))} ` +
                        `never applied, have ${JSON.stringify(monitors().map(g => [g.x, g.y, g.width, g.height]))}`));
            }, e => fail(`cannot apply the layout: ${e}`));
        }, e => fail(`cannot read the layout: ${e}`));
    }

    _capture(at, then, fail) {
        const r = this._r();
        this._point(at);
        const there = () => {
            const [x, y] = global.get_pointer();
            if (x === Math.round(at.x) && y === Math.round(at.y))
                return true;
            this._point(at);
            return false;
        };
        waitFor(() => there() && !r._busy && r._server?.ready, () => painted(() => {
            const original = r._frameMonitors;
            this._sent = null;
            r._frameMonitors = texture => (this._sent = original.call(r, texture));
            try {
                r._capture();
            } catch (e) {
                r._frameMonitors = original;
                fail(`the capture threw: ${e.message}`);
                return;
            }
            waitFor(() => r._overlay?.shown && r._overlay.framePath, () => {
                r._frameMonitors = original;
                then(r._overlay);
            }, () => {
                r._frameMonitors = original;
                fail('the overlay did not show');
            });
        }), () => fail('no warm spare in time'));
    }

    _point(at) {
        this._device().notify_absolute_motion(GLib.get_monotonic_time(), at.x, at.y);
    }

    _drag(from, to, then) {
        St.Clipboard.get_default().set_text(St.ClipboardType.CLIPBOARD, 'rayshot-soak-marker');
        const p = this._device();
        const now = () => GLib.get_monotonic_time();
        const steps = [() => p.notify_button(now(), Clutter.BUTTON_PRIMARY, Clutter.ButtonState.PRESSED)];
        for (let i = 1; i < 8; i++) {
            const t = i / 8;
            steps.push(() => p.notify_absolute_motion(now(),
                from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t));
        }
        steps.push(() => {
            p.notify_absolute_motion(now(), to.x, to.y);
            p.notify_button(now(), Clutter.BUTTON_PRIMARY, Clutter.ButtonState.RELEASED);
            const k = this._d._keyboard;
            k.notify_keyval(now(), Clutter.KEY_Return, Clutter.KeyState.PRESSED);
            k.notify_keyval(now(), Clutter.KEY_Return, Clutter.KeyState.RELEASED);
        });
        sequence(steps, then);
    }

    _enterAndRead(then, fail) {
        const r = this._r();
        waitFor(() => !r._busy, () => later(400, () => readPng(then, fail)),
            () => fail('the overlay did not close after enter'));
    }

    _windowsFit(overlay, geoms) {
        const rects = [...overlay.windows.keys()].map(w => w.get_frame_rect());
        const wrong = geoms.filter(g => !rects.some(r =>
            r.x === g.x && r.y === g.y && r.width === g.width && r.height === g.height));
        if (rects.length !== geoms.length || wrong.length)
            return `${rects.length} overlay windows at ${JSON.stringify(rects.map(r => [r.x, r.y, r.width, r.height]))} ` +
                `for monitors ${JSON.stringify(geoms.map(g => [g.x, g.y, g.width, g.height]))}`;
        return null;
    }

    _checkBox(pb, scale, origin, mark, rgb) {
        const x0 = (mark.x - origin.x) * scale;
        const y0 = (mark.y - origin.y) * scale;
        const x1 = x0 + MARK * scale;
        const y1 = y0 + MARK * scale;
        const cy = (y0 + y1) / 2;
        const cx = (x0 + x1) / 2;
        const probes = [
            ['centre', cx, cy, true],
            ['inside left edge', x0 + EDGE, cy, true],
            ['inside right edge', x1 - 1 - EDGE, cy, true],
            ['inside top edge', cx, y0 + EDGE, true],
            ['inside bottom edge', cx, y1 - 1 - EDGE, true],
            ['outside left edge', x0 - 1 - EDGE, cy, false],
            ['outside right edge', x1 + EDGE, cy, false],
            ['outside top edge', cx, y0 - 1 - EDGE, false],
            ['outside bottom edge', cx, y1 + EDGE, false],
        ];
        for (const [what, x, y, inside] of probes) {
            const got = pixel(pb, x, y);
            if (near(got, rgb) !== inside)
                return `${hex(rgb)} square: ${what} at ${Math.round(x)},${Math.round(y)} is ${hex(got)}`;
        }
        return null;
    }

    _layoutCase(kind, next) {
        const name = `display ${kind}`;
        const fail = why => this._fail(name, why, next);
        this._apply(kind, placed => {
            const geoms = monitors();
            const marks = geoms.map(g => ({
                x: g.x + Math.floor(g.width / 2) - MARK / 2,
                y: g.y + Math.floor(g.height / 2) - MARK / 2,
            }));
            marks.forEach((m, i) => this._mark(m.x, m.y, COLOURS[i % COLOURS.length]));
            const seam = this._seam(geoms);
            if (seam)
                seam.marks.forEach((m, i) => this._mark(m.x, m.y, COLOURS[(i + 1) % COLOURS.length]));
            const layout = placed.map(p => `${p.w}x${p.h}@${p.scale}${p.transform ? ' rotated' : ''}`).join(' + ');
            const sizes = [];
            const onMonitor = i => {
                if (i === marks.length) {
                    const detail = `${layout}, png ${sizes.join(' ')}`;
                    if (!seam) {
                        this._clearMarks();
                        this._d._result(name, true, detail);
                        next();
                        return;
                    }
                    this._acrossSeam(name, seam, this._scale, detail, next);
                    return;
                }
                const m = marks[i];
                const origin = {x: m.x - PAD, y: m.y - PAD};
                const end = {x: m.x + MARK + PAD, y: m.y + MARK + PAD};
                const where = `monitor ${i} (${layout})`;
                this._capture(origin, overlay => {
                    this._scale = overlay.texture.get_width() / global.stage.width;
                    const problem = this._windowsFit(overlay, geoms);
                    if (problem) {
                        fail(`${where}: ${problem}`);
                        return;
                    }
                    const sent = this._sent ?? [];
                    if (sent.length !== geoms.length) {
                        fail(`${where}: the overlay was told about ${sent.length} of ${geoms.length} monitors: ${sent.join(' ')}`);
                        return;
                    }
                    this._drag(origin, end, () => this._enterAndRead(pb => {
                        const want = (MARK + 2 * PAD) * this._scale;
                        const size = `${pb.get_width()}x${pb.get_height()}`;
                        if (Math.abs(pb.get_width() - want) > 2 || Math.abs(pb.get_height() - want) > 2) {
                            fail(`${where}: png ${size}, want about ${Math.round(want)} square ` +
                                `(frame ${overlay.texture.get_width()}x${overlay.texture.get_height()})`);
                            return;
                        }
                        const off = this._checkBox(pb, this._scale, origin, m, COLOURS[i % COLOURS.length]);
                        if (off) {
                            fail(`${where}: png ${size}: ${off}`);
                            return;
                        }
                        sizes.push(size);
                        onMonitor(i + 1);
                    }, why => fail(`${where}: ${why}`)));
                }, why => fail(`${where}: ${why}`));
            };
            onMonitor(0);
        }, fail);
    }

    _seam(geoms) {
        if (geoms.length < 2)
            return null;
        const [a, b] = geoms;
        if (a.x + a.width === b.x || b.x + b.width === a.x) {
            const left = a.x < b.x ? a : b;
            const x = left.x + left.width;
            const y = Math.max(a.y, b.y) + 100;
            return {from: {x: x - 100, y}, to: {x: x + 100, y: y + 140},
                marks: [{x: x - 70, y: y + 50}, {x: x + 30, y: y + 50}]};
        }
        if (a.y + a.height === b.y || b.y + b.height === a.y) {
            const top = a.y < b.y ? a : b;
            const y = top.y + top.height;
            const x = Math.max(a.x, b.x) + 100;
            return {from: {x, y: y - 100}, to: {x: x + 140, y: y + 100},
                marks: [{x: x + 50, y: y - 70}, {x: x + 50, y: y + 30}]};
        }
        return null;
    }

    _acrossSeam(name, seam, scale, detail, next) {
        const fail = why => this._fail(name, `${detail}; across the seam: ${why}`, next);
        this._capture(seam.from, () => this._drag(seam.from, seam.to, () => this._enterAndRead(pb => {
            const w = (seam.to.x - seam.from.x) * scale;
            const h = (seam.to.y - seam.from.y) * scale;
            if (Math.abs(pb.get_width() - w) > 2 || Math.abs(pb.get_height() - h) > 2) {
                fail(`png ${pb.get_width()}x${pb.get_height()}, want about ${Math.round(w)}x${Math.round(h)}`);
                return;
            }
            for (const [i, m] of seam.marks.entries()) {
                const off = this._checkBox(pb, scale, seam.from, m, COLOURS[(i + 1) % COLOURS.length]);
                if (off) {
                    fail(off);
                    return;
                }
            }
            this._clearMarks();
            this._d._result(name, true, `${detail}; across the seam too`);
            next();
        }, fail)), fail);
    }

    _fullscreen(next) {
        const name = 'a fullscreen app is captured as it looks';
        const fill = [255, 0, 255];
        let helper = null;
        const findWindow = () => global.get_window_actors().map(a => a.get_meta_window())
            .find(w => w?.get_title() === 'rayshot-soak-helper');
        const finish = (ok, detail) => {
            helper?.force_exit();
            waitFor(() => !findWindow(), () => {
                this._d._result(name, ok, detail);
                next();
            }, () => {
                this._d._result(name, false, `${detail}; the helper window did not close`);
                next();
            });
        };
        const fail = why => {
            if (this._r()._busy)
                this._d._endSession(() => finish(false, why));
            else
                finish(false, why);
        };
        try {
            const launcher = new Gio.SubprocessLauncher();
            launcher.setenv('RAYSHOT_SOAK_HELPER_FULLSCREEN', `rgb(${fill.join(',')})`, true);
            helper = launcher.spawnv(['gjs', '-m', `${this._d.path}/helper-window.js`]);
        } catch (e) {
            fail(`cannot start the helper: ${e.message}`);
            return;
        }
        waitFor(() => findWindow()?.is_fullscreen() && findWindow().get_compositor_private(), () => later(500, () => {
            const g = global.display.get_monitor_geometry(findWindow().get_monitor());
            const from = {x: g.x + Math.floor(g.width / 2) - 100, y: g.y + Math.floor(g.height / 2) - 100};
            const to = {x: from.x + 200, y: from.y + 200};
            this._capture(from, overlay => this._drag(from, to, () => this._enterAndRead(pb => {
                const want = 200 * overlay.texture.get_width() / global.stage.width;
                if (Math.abs(pb.get_width() - want) > 2 || Math.abs(pb.get_height() - want) > 2) {
                    finish(false, `png ${pb.get_width()}x${pb.get_height()}, want about ${Math.round(want)} square ` +
                        `on monitor ${JSON.stringify([g.x, g.y, g.width, g.height])}`);
                    return;
                }
                const got = [pixel(pb, pb.get_width() / 2, pb.get_height() / 2),
                    pixel(pb, 5, 5), pixel(pb, pb.get_width() - 6, pb.get_height() - 6)];
                const ok = got.every(c => near(c, fill));
                finish(ok, `png ${pb.get_width()}x${pb.get_height()}, pixels ${got.map(hex).join(' ')}, ` +
                    `want ${hex(fill)}`);
            }, fail)), fail);
        }), () => fail('the helper window never went fullscreen'));
    }

    _shapeEnter(next) {
        const name = 'enter with a shape selected lets go of the shape first';
        const r = this._r();
        const fail = why => this._fail(name, why, next);
        const k = this._d._keyboard;
        const now = () => GLib.get_monotonic_time();
        const tap = v => {
            k.notify_keyval(now(), v, Clutter.KeyState.PRESSED);
            k.notify_keyval(now(), v, Clutter.KeyState.RELEASED);
        };
        const nudge = () => {
            k.notify_keyval(now(), Clutter.KEY_Shift_L, Clutter.KeyState.PRESSED);
            tap(Clutter.KEY_Right);
            k.notify_keyval(now(), Clutter.KEY_Shift_L, Clutter.KeyState.RELEASED);
        };
        const marker = 'rayshot-soak-marker';
        const clipboard = St.Clipboard.get_default();
        const area = {x: 200, y: 200};
        const from = {x: 400, y: 400};
        const to = {x: 600, y: 550};
        this._capture(from, overlay => {
            try {
                overlay.proc.get_stdin_pipe().write_all(
                    new TextEncoder().encode(`select ${overlay.session} ${area.x} ${area.y} 900 800\n`), null);
            } catch (e) {
                fail(`cannot select: ${e.message}`);
                return;
            }
            clipboard.set_text(St.ClipboardType.CLIPBOARD, marker);
            const p = this._device();
            const press = () => p.notify_button(now(), Clutter.BUTTON_PRIMARY, Clutter.ButtonState.PRESSED);
            const release = () => p.notify_button(now(), Clutter.BUTTON_PRIMARY, Clutter.ButtonState.RELEASED);
            const move = at => p.notify_absolute_motion(now(), at.x, at.y);
            sequence([
                () => tap(Clutter.KEY_r),
                () => move(from),
                press,
                () => move({x: 500, y: 480}),
                () => move(to),
                release,
                () => tap(Clutter.KEY_s),
                () => move({x: from.x, y: 470}),
                press,
                release,
                nudge,
                nudge,
                nudge,
                () => tap(Clutter.KEY_Return),
            ], () => later(600, () => clipboard.get_text(St.ClipboardType.CLIPBOARD, (_c, text) => {
                if (!r._busy || text !== marker) {
                    this._d._result(name, false, `the first enter copied (clipboard ${text === marker ? 'unchanged' : 'changed'})`);
                    if (r._busy)
                        this._d._endSession(next);
                    else
                        next();
                    return;
                }
                tap(Clutter.KEY_Return);
                waitFor(() => !r._busy, () => later(400, () => readPng(pb => {
                    const scale = overlay.texture.get_width() / global.stage.width;
                    const y = (470 - area.y) * scale;
                    const moved = pixel(pb, (from.x + 30 - area.x) * scale, y);
                    const old = pixel(pb, (from.x - area.x) * scale - 4, y);
                    const red = moved !== null && moved[0] > 180 && moved[1] < 120 && moved[2] < 120;
                    const stayed = old !== null && old[0] > 180 && old[1] < 120 && old[2] < 120;
                    const ok = red && !stayed;
                    this._d._result(name, ok, ok
                        ? 'the first enter only let go of the shape, the second copied it nudged 30 px'
                        : `after the second enter the rect edge is ${hex(moved)} at the nudged spot and ${hex(old)} where it was`);
                    next();
                }, why => {
                    this._d._result(name, false, `the second enter: ${why}`);
                    next();
                })), () => {
                    this._d._result(name, false, 'the second enter did not copy');
                    this._d._endSession(next);
                });
            })));
        }, fail);
    }

    _textKey(keyval, next) {
        const name = `${keyval === Clutter.KEY_Return ? 'enter' : 'escape'} in a text box only ends the text`;
        const r = this._r();
        const fail = why => this._fail(name, why, next);
        const k = this._d._keyboard;
        const now = () => GLib.get_monotonic_time();
        const tap = v => {
            k.notify_keyval(now(), v, Clutter.KeyState.PRESSED);
            k.notify_keyval(now(), v, Clutter.KeyState.RELEASED);
        };
        const spot = {x: 300, y: 300};
        this._capture(spot, overlay => {
            try {
                overlay.proc.get_stdin_pipe().write_all(
                    new TextEncoder().encode(`select ${overlay.session} 200 200 900 700\n`), null);
            } catch (e) {
                fail(`cannot select: ${e.message}`);
                return;
            }
            const p = this._device();
            sequence([
                () => tap(Clutter.KEY_t),
                () => p.notify_button(now(), Clutter.BUTTON_PRIMARY, Clutter.ButtonState.PRESSED),
                () => p.notify_button(now(), Clutter.BUTTON_PRIMARY, Clutter.ButtonState.RELEASED),
                () => tap(Clutter.KEY_a),
                () => tap(Clutter.KEY_b),
                () => {
                    p.notify_absolute_motion(now(), spot.x + 3, spot.y + 3);
                    later(1, () => tap(keyval));
                },
            ], () => later(800, () => {
                if (!r._busy) {
                    this._d._result(name, false, 'the editor closed');
                    next();
                    return;
                }
                this._d._result(name, true, 'the editor stayed open');
                this._d._endSession(next);
            }));
        }, fail);
    }

    _russian(next) {
        const name = 'ctrl c with the russian layout';
        const fail = why => {
            getInputSourceManager().inputSources[0]?.activate(false);
            this._fail(name, why, next);
        };
        const sources = getInputSourceManager().inputSources;
        const ru = Object.values(sources).find(s => s.id === 'ru');
        if (!ru) {
            fail('no russian input source configured');
            return;
        }
        ru.activate(false);
        waitFor(() => getInputSourceManager().currentSource?.id === 'ru', () => {
            this._capture({x: 50, y: 50}, overlay => {
                const r = this._r();
                St.Clipboard.get_default().set_text(St.ClipboardType.CLIPBOARD, 'rayshot-soak-marker');
                try {
                    overlay.proc.get_stdin_pipe().write_all(
                        new TextEncoder().encode(`select ${overlay.session} 100 100 300 200\n`), null);
                } catch (e) {
                    fail(`cannot select: ${e.message}`);
                    return;
                }
                later(300, () => {
                    const k = this._d._keyboard;
                    const t = GLib.get_monotonic_time();
                    k.notify_key(t, KEY_LEFTCTRL, Clutter.KeyState.PRESSED);
                    k.notify_key(t + 1000, KEY_C, Clutter.KeyState.PRESSED);
                    k.notify_key(t + 2000, KEY_C, Clutter.KeyState.RELEASED);
                    k.notify_key(t + 3000, KEY_LEFTCTRL, Clutter.KeyState.RELEASED);
                    waitFor(() => !r._busy, () => later(400, () => readPng(pb => {
                        getInputSourceManager().inputSources[0]?.activate(false);
                        this._d._result(name, true, `png ${pb.get_width()}x${pb.get_height()} on the clipboard`);
                        next();
                    }, fail)), () => fail('ctrl+c did not close the overlay'));
                });
            }, fail);
        }, () => fail('the russian layout did not become active'));
    }

    _spam(next) {
        const name = 'the hotkey pressed five times in a row';
        const r = this._r();
        const fail = why => this._fail(name, why, next);
        waitFor(() => !r._busy && r._server?.ready, () => {
            const before = r._session;
            const press = i => {
                if (i === 5) {
                    waitFor(() => r._overlay?.shown && r._overlay.framePath, () => {
                        const sessions = r._session - before;
                        const freezes = Main.layoutManager.uiGroup.get_children()
                            .filter(a => a === r._frozen?.actor).length;
                        const windows = r._overlay.windows.size;
                        this._d._endSession(() => {
                            if (sessions !== 1 || windows !== global.display.get_n_monitors()) {
                                this._d._result(name, false,
                                    `${sessions} sessions, ${windows} overlay windows, ${freezes} freeze`);
                                next();
                                return;
                            }
                            this._capture({x: 50, y: 50}, () => this._d._endSession(() => {
                                this._d._result(name, true,
                                    `one session with ${windows} windows, the next capture works`);
                                next();
                            }), fail);
                        });
                    }, () => fail('the overlay did not show'));
                    return;
                }
                r._capture();
                later(15, () => press(i + 1));
            };
            painted(() => press(0));
        }, () => fail('no warm spare in time'));
    }
}
