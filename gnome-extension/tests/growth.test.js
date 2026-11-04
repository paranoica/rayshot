import {growth} from './soak/growth.js';

let failed = 0;
let total = 0;

function check(name, actual, expected) {
    total++;
    if (Object.is(actual, expected)) {
        print(`ok   ${name}`);
    } else {
        failed++;
        print(`FAIL ${name}: got ${actual}, want ${expected}`);
    }
}

const s = (...values) => values.map((v, i) => ({i: i * 10, v, server_pid: 7}));

check('no samples', growth([], 'v'), null);
check('one sample', growth(s(100), 'v'), null);
check('two samples use the first as baseline', growth(s(100, 150), 'v'), 50);
check('warmup sample is skipped', growth(s(100, 120, 130), 'v'), 10);
check('missing values are skipped', growth(s(100, null, 120, 125), 'v'), 5);
check('shrinking is negative', growth(s(100, 200, 150), 'v'), -50);
check('all missing', growth(s(null, null, null), 'v'), null);
check('warmup of three samples', growth(s(100, 150, 180, 190, 191, 192), 'v', 3), 2);
check('warmup longer than the run uses the previous sample',
    growth(s(100, 150, 180), 'v', 9), 30);
check('warmup zero uses the first sample', growth(s(100, 150, 180), 'v', 0), 80);

// Software rendering keeps textures in RAM and frees them in bursts, so the
// shell saws up and down; only a rising floor is a leak.
check('sawtooth that returns to its floor is not growth',
    growth(s(300, 360, 430, 300, 440, 310, 450), 'v', 1) <= 0, true);
check('sawtooth with a rising floor is growth',
    growth(s(300, 360, 430, 330, 470, 360, 500, 390), 'v', 1) > 0, true);
check('a steady leak counts over the whole run',
    growth(s(0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110), 'v', 0), 110);

// The verdict must judge warm and cold sessions apart: a leak that starts in
// the cold phase would hide behind flat warm samples in one combined series.
const GLib = imports.gi.GLib;
const here = GLib.path_get_dirname(GLib.filename_from_uri(import.meta.url)[0]);
const result = {
    error: null,
    scenarios: [],
    samples: [
        ...[0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100].map(i => ({phase: 'warm', i, shell_rss_kb: 400000})),
        ...[0, 10, 20].map((i, k) => ({phase: 'cold', i, shell_rss_kb: 400000 + k * 40000})),
    ],
};
const Gio = imports.gi.Gio;
function verdictStatus(samples) {
    const file = GLib.build_filenamev([GLib.get_tmp_dir(), `rayshot-verdict-test-${GLib.random_int()}.json`]);
    GLib.file_set_contents(file, JSON.stringify({...result, samples}));
    const verdict = Gio.Subprocess.new(
        ['gjs', '-m', `${here}/soak/verdict.js`, file, '16384', '16', '16384'],
        Gio.SubprocessFlags.STDOUT_SILENCE);
    verdict.wait(null);
    GLib.unlink(file);
    return verdict.get_exit_status();
}
const flat = result.samples.map(sample => ({...sample, shell_rss_kb: 400000}));
check('a flat run passes the verdict', verdictStatus(flat), 0);
check('a leak only in the cold phase fails the verdict', verdictStatus(result.samples) !== 0, true);

print(`${total - failed}/${total} passed`);
if (failed)
    imports.system.exit(1);
