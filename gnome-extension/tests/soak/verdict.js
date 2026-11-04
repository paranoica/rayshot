import GLib from 'gi://GLib';
import System from 'system';

import {growth} from './growth.js';

const [path, serverRssLimit, serverVramLimit, shellRssLimit] = ARGV;
const [, bytes] = GLib.file_get_contents(path);
const {error, samples, scenarios = [], driverCrashes = 0} = JSON.parse(new TextDecoder().decode(bytes));

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
// The shell settles over its first ~30 sessions (startup garbage collection).
// Every session now runs in a fresh spare, so the spare metrics catch drift in
// the per-process footprint, not a leak inside one process.
const WARMUP_SAMPLES = 3;
const spares = Math.max(0, ...samples.map(s => s.serve_processes ?? 0));
report('live spare processes at once', spares, 1, '');
report('spare rss drift', growth(warm, 'server_rss_kb', WARMUP_SAMPLES), Number(serverRssLimit), 'kB');
report('spare vram drift', growth(warm, 'server_vram_mib', WARMUP_SAMPLES),
    Number(serverVramLimit), 'MiB');
// Apart, so a leak that starts in one phase cannot hide behind the other.
report('shell rss growth in warm sessions', growth(warm, 'shell_rss_kb', WARMUP_SAMPLES),
    Number(shellRssLimit), 'kB');
// The switch to cold sessions settles once, like the start of the run.
report('shell rss growth in cold sessions', growth(cold, 'shell_rss_kb', 1),
    Number(shellRssLimit), 'kB');
report('spares lost to a GPU driver crash', driverCrashes, 3, '');
for (const {name, ok, detail} of scenarios) {
    failed ||= !ok;
    print(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${detail}`);
}

System.exit(failed ? 1 : 0);
