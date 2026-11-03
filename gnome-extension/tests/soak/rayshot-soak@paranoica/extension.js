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
            server_rss_kb: server ? rssKb(server.pid) : null,
            server_vram_mib: server ? vramMib(server.pid) : null,
        });
    }

    _run(phase, total, i) {
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
        const r = this._rayshot;
        r._capture();
        waitFor(() => r._overlay?.shown && r._overlay.framePath, () => {
            const o = r._overlay;
            const line = o.server ? `cancel ${o.session}\n` : 'quit\n';
            try {
                o.proc.get_stdin_pipe().write_all(new TextEncoder().encode(line), null);
            } catch (e) {
                this._finish(`${phase} ${i}: cannot end the session: ${e.message}`);
                return;
            }
            waitFor(() => !r._busy, () => this._run(phase, total, i + 1),
                () => this._finish(`${phase} ${i}: session did not end`));
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
