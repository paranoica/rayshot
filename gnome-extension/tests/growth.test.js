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

print(`${total - failed}/${total} passed`);
if (failed)
    imports.system.exit(1);
