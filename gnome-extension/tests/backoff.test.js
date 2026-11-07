import {MAX_FAILURES, respawnDelay, shouldRespawn} from '../rayshot@paranoica/backoff.js';

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
check('gives up after five failures', MAX_FAILURES, 5);
check('respawns after the first failure', shouldRespawn(1), true);
check('respawns after the fourth failure', shouldRespawn(4), true);
check('stops at the fifth failure', shouldRespawn(5), false);
check('stays stopped past the limit', shouldRespawn(6), false);
check('respawns with no failures', shouldRespawn(0), true);

print(`${total - failed}/${total} passed`);
if (failed)
    imports.system.exit(1);
