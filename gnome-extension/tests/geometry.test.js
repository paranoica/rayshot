import System from 'system';

import {pickClickPoint} from '../rayshot@paranoica/geometry.js';

const area = {x: 0, y: 27, width: 1920, height: 1053};
const rect = (x, y, width, height) => ({x, y, width, height});

const cases = [
    ['no popups -> work area center', area, [], {x: 960, y: 553}],
    ['popup over center -> top-left inset', area, [rect(900, 500, 200, 200)], {x: 8, y: 35}],
    ['center and top-left covered -> top-right inset', area,
        [rect(900, 500, 200, 200), rect(0, 27, 100, 100)], {x: 1911, y: 35}],
    ['center, top corners covered -> bottom-left inset', area,
        [rect(900, 500, 200, 200), rect(0, 0, 1920, 100)], {x: 8, y: 1071}],
    ['only bottom-right free', area,
        [rect(0, 0, 1920, 1000), rect(0, 1000, 1000, 80)], {x: 1911, y: 1071}],
    ['popup covers whole area -> null', area, [rect(0, 0, 1920, 1080)], null],
    ['popup ending exactly at center does not cover it', area,
        [rect(0, 0, 960, 1080)], {x: 960, y: 553}],
    ['popup starting exactly at center covers it', area,
        [rect(960, 0, 10, 1080)], {x: 8, y: 35}],
    ['zero-size popup covers nothing', area, [rect(960, 553, 0, 0)], {x: 960, y: 553}],
    ['second monitor offset', rect(1920, 0, 1920, 1080), [], {x: 2880, y: 540}],
    ['popup on other monitor is ignored', rect(1920, 0, 1920, 1080),
        [rect(900, 500, 200, 200)], {x: 2880, y: 540}],
    ['zero-width area -> null', rect(0, 0, 0, 1080), [], null],
    ['zero-height area -> null', rect(0, 0, 1920, 0), [], null],
];

let failed = 0;
for (const [name, a, popups, expected] of cases) {
    const got = pickClickPoint(a, popups);
    const ok = JSON.stringify(got) === JSON.stringify(expected);
    if (!ok)
        failed++;
    print(`${ok ? 'ok  ' : 'FAIL'} ${name}${ok ? '' : `: got ${JSON.stringify(got)}, want ${JSON.stringify(expected)}`}`);
}
print(`${cases.length - failed}/${cases.length} passed`);
System.exit(failed ? 1 : 0);
