import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

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
    const dir = GLib.Dir.open('/proc', 0);
    for (let name; (name = dir.read_name());) {
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
    dir.close();
    return count;
}

function waitFor(cond, then, fail) {
    const started = GLib.get_monotonic_time();
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, TICK_MS, () => {
        if (cond()) {
            then();
            return GLib.SOURCE_REMOVE;
        }
        if (GLib.get_monotonic_time() - started > STEP_TIMEOUT_MS * 1000) {
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
                this._finish(null);
            }
            return;
        }
        r._capture();
        waitFor(() => r._overlay?.shown && r._overlay.framePath, () => {
            const o = r._overlay;
            if (phase === 'warm' && !o.server) {
                this._finish(`warm ${i}: session took the one-shot path`);
                return;
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

    _finish(error) {
        this._rayshot?._settings?.set_boolean('warm-overlay', true);
        const result = JSON.stringify({error, samples: this._samples}, null, 1);
        if (this._out)
            GLib.file_set_contents(this._out, result);
        console.log(`rayshot-soak: done ${error ?? 'ok'}`);
    }
}
