import GLib from 'gi://GLib';
import System from 'system';

import {growth, restarts} from './growth.js';

const [path, serverRssLimit, serverVramLimit, shellRssLimit] = ARGV;
const [, bytes] = GLib.file_get_contents(path);
const {error, samples} = JSON.parse(new TextDecoder().decode(bytes));

let failed = false;
function report(name, value, limit, unit) {
    if (value === null) {
        print(`skip ${name}: not measured`);
        return;
    }
    const ok = value <= limit;
    failed ||= !ok;
    print(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${value} ${unit} (limit ${limit})`);
}

if (error) {
    print(`FAIL driver: ${error}`);
    failed = true;
}

const warm = samples.filter(s => s.phase === 'warm');
const cold = samples.filter(s => s.phase === 'cold');
print(`sessions: warm ${warm.at(-1)?.i ?? 0}, cold ${cold.at(-1)?.i ?? 0}`);
report('server restarts', restarts(warm), 0, '');
// Both processes settle over the first ~30 sessions: the server warms shader
// and glyph caches, the shell runs its first garbage collections.
const WARMUP_SAMPLES = 3;
report('server rss growth', growth(warm, 'server_rss_kb', WARMUP_SAMPLES),
    Number(serverRssLimit), 'kB');
report('server vram growth', growth(warm, 'server_vram_mib', WARMUP_SAMPLES),
    Number(serverVramLimit), 'MiB');
report('shell rss growth', growth(samples, 'shell_rss_kb', WARMUP_SAMPLES),
    Number(shellRssLimit), 'kB');

System.exit(failed ? 1 : 0);
