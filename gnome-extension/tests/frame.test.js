import {frameHeader} from '../rayshot@paranoica/frame.js';

let failed = 0;
let total = 0;

function check(name, fn) {
    total++;
    try {
        fn();
        print(`ok   ${name}`);
    } catch (e) {
        failed++;
        print(`FAIL ${name}: ${e.message}`);
    }
}

function eq(actual, expected) {
    const a = Array.from(actual);
    if (a.length !== expected.length || a.some((v, i) => v !== expected[i]))
        throw new Error(`got [${a}], want [${expected}]`);
}

function throws(fn) {
    let threw = false;
    try {
        fn();
    } catch (_) {
        threw = true;
    }
    if (!threw)
        throw new Error('did not throw');
}

const MAGIC = [0x52, 0x53, 0x46, 0x31];

check('small frame', () => eq(frameHeader(2, 1, 8), [...MAGIC, 2, 0, 0, 0, 1, 0, 0, 0, 8, 0, 0, 0]));
check('little endian fields', () => eq(frameHeader(0x01020304, 0x0a0b0c0d, 0x7f000001),
    [...MAGIC, 4, 3, 2, 1, 0x0d, 0x0c, 0x0b, 0x0a, 1, 0, 0, 0x7f]));
check('max u32', () => eq(frameHeader(0xffffffff, 1, 0xffffffff),
    [...MAGIC, 255, 255, 255, 255, 1, 0, 0, 0, 255, 255, 255, 255]));
check('two monitors', () => eq(frameHeader(3840, 1080, 15360),
    [...MAGIC, 0x00, 0x0f, 0, 0, 0x38, 0x04, 0, 0, 0x00, 0x3c, 0, 0]));
check('rejects negative', () => throws(() => frameHeader(-1, 1, 4)));
check('rejects fraction', () => throws(() => frameHeader(1.5, 1, 8)));
check('rejects above u32', () => throws(() => frameHeader(0x100000000, 1, 8)));
check('rejects NaN', () => throws(() => frameHeader(NaN, 1, 8)));

print(`${total - failed}/${total} passed`);
if (failed)
    imports.system.exit(1);
