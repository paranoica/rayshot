import System from 'system';

import {frameMonitors, pickClickPoint} from '../rayshot@paranoica/geometry.js';

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
const stage = (width, height) => ({width, height});
const at = (...geoms) => connector => {
    const i = Number(connector.split('-').pop()) - 1;
    return i >= 0 && i < geoms.length ? i : -1;
};
const frameCases = [
    ['one monitor at 1x', ['DP-1'], at(rect(0, 0, 1920, 1080)), [rect(0, 0, 1920, 1080)],
        stage(1920, 1080), stage(1920, 1080), ['DP-1:0,0,1920,1080']],
    ['200% and 100% side by side, frame at 2x', ['DP-1', 'DP-2'], at(0, 1),
        [rect(0, 0, 960, 540), rect(960, 0, 1920, 1080)], stage(5760, 2160), stage(2880, 1080),
        ['DP-1:0,0,1920,1080', 'DP-2:1920,0,3840,2160']],
    ['rotated monitor keeps its rotated size', ['DP-1', 'DP-2'], at(0, 1),
        [rect(0, 0, 1920, 1080), rect(1920, 0, 1080, 1920)], stage(3000, 1920), stage(3000, 1920),
        ['DP-1:0,0,1920,1080', 'DP-2:1920,0,1080,1920']],
    ['150% rounds to whole pixels', ['DP-1', 'DP-2'], at(0, 1),
        [rect(0, 0, 1280, 720), rect(1280, 0, 1280, 720)], stage(3840, 1080), stage(2560, 720),
        ['DP-1:0,0,1920,1080', 'DP-2:1920,0,1920,1080']],
    ['an inexact ratio leaves no gap and no overlap', ['DP-1', 'DP-2'], at(0, 1),
        [rect(0, 0, 683, 384), rect(683, 0, 683, 384)], stage(2733, 768), stage(1366, 384),
        ['DP-1:0,0,1367,768', 'DP-2:1367,0,1366,768']],
    ['a region never runs past the frame', ['DP-1'], at(0),
        [rect(0, 0, 1001, 500)], stage(1500, 750), stage(1000, 500), ['DP-1:0,0,1500,750']],
    ['swapped monitors follow their connectors, not stale positions', ['DP-1', 'DP-2'],
        c => ({'DP-1': 1, 'DP-2': 0})[c], [rect(0, 0, 1920, 1080), rect(1920, 0, 1920, 1080)],
        stage(3840, 1080), stage(3840, 1080), ['DP-1:1920,0,1920,1080', 'DP-2:0,0,1920,1080']],
    ['an unplugged connector is left out', ['DP-1', 'DP-9'], c => (c === 'DP-1' ? 0 : -1),
        [rect(0, 0, 1920, 1080)], stage(1920, 1080), stage(1920, 1080), ['DP-1:0,0,1920,1080']],
    ['a connector that would break the line is left out', ['bad name', 'a:b', ''], () => 0,
        [rect(0, 0, 1920, 1080)], stage(1920, 1080), stage(1920, 1080), []],
    ['an empty monitor is left out', ['DP-1'], () => 0,
        [rect(0, 0, 0, 1080)], stage(1920, 1080), stage(1920, 1080), []],
    ['no connectors known yet', [], () => 0, [rect(0, 0, 1920, 1080)],
        stage(1920, 1080), stage(1920, 1080), []],
];
for (const [name, connectors, indexOf, monitors, frame, st, expected] of frameCases) {
    const got = frameMonitors(connectors, indexOf, monitors, frame, st);
    const ok = JSON.stringify(got) === JSON.stringify(expected);
    if (!ok)
        failed++;
    print(`${ok ? 'ok  ' : 'FAIL'} ${name}${ok ? '' : `: got ${JSON.stringify(got)}, want ${JSON.stringify(expected)}`}`);
}
const total = cases.length + frameCases.length;
print(`${total - failed}/${total} passed`);
System.exit(failed ? 1 : 0);
