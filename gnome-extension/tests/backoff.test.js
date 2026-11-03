import {respawnDelay} from '../rayshot@paranoica/backoff.js';

let failed = 0;
let total = 0;

function check(name, actual, expected) {
    total++;
    if (actual === expected) {
        print(`ok   ${name}`);
    } else {
        failed++;
        print(`FAIL ${name}: got ${actual}, want ${expected}`);
    }
}

check('first respawn', respawnDelay(0), 250);
check('second respawn', respawnDelay(1), 500);
check('doubles', respawnDelay(2), 1000);
check('last below cap', respawnDelay(6), 16000);
check('first capped', respawnDelay(7), 30000);
check('stays capped', respawnDelay(50), 30000);
check('huge count stays finite', respawnDelay(5000), 30000);
check('negative counts as zero', respawnDelay(-3), 250);
check('NaN counts as zero', respawnDelay(NaN), 250);
check('fraction rounds down', respawnDelay(1.9), 500);

print(`${total - failed}/${total} passed`);
if (failed)
    imports.system.exit(1);
