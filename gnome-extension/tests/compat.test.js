import {PROTOCOL_VERSION, readyVersion, resolveBin} from '../rayshot@paranoica/compat.js';

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

const exists = path => ['/opt/dev/rayshot', '/usr/bin/rayshot'].includes(path);

check('env override wins', resolveBin('/opt/dev/rayshot', '/usr/bin/rayshot', '/usr/bin/rayshot', exists), '/opt/dev/rayshot');
check('relative env is ignored', resolveBin('rayshot', '/opt/dev/rayshot', null, exists), '/opt/dev/rayshot');
check('missing env file is ignored', resolveBin('/nope/rayshot', null, '/usr/bin/rayshot', exists), '/usr/bin/rayshot');
check('setting beats PATH', resolveBin(null, '/opt/dev/rayshot', '/usr/bin/rayshot', exists), '/opt/dev/rayshot');
check('empty setting falls back to PATH', resolveBin(null, '', '/usr/bin/rayshot', exists), '/usr/bin/rayshot');
check('stale setting falls back to PATH', resolveBin(null, '/old/rayshot', '/usr/bin/rayshot', exists), '/usr/bin/rayshot');
check('nothing found', resolveBin(null, '', null, exists), null);
check('empty env is ignored', resolveBin('', '', '/usr/bin/rayshot', exists), '/usr/bin/rayshot');

check('current protocol', readyVersion(`ready ${PROTOCOL_VERSION}`), PROTOCOL_VERSION);
check('legacy bare ready', readyVersion('ready'), 0);
check('future protocol', readyVersion('ready 7'), 7);
check('not a ready line', readyVersion('closed 3'), null);
check('garbage version', readyVersion('ready x'), null);
check('extra fields', readyVersion('ready 1 2'), null);
check('negative version', readyVersion('ready -1'), null);

print(`${total - failed}/${total} passed`);
if (failed)
    imports.system.exit(1);
