import {PROTOCOL_VERSION, readyVersion, resolveBin, unredirect, defaultSeat, persistentNotice} from '../rayshot@paranoica/compat.js';

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

const calls = [];
const display = {name: 'display'};
const Meta = {
    disable_unredirect_for_display: d => calls.push(`meta off ${d.name}`),
    enable_unredirect_for_display: d => calls.push(`meta on ${d.name}`),
};
const newer = {
    disable_unredirect: () => calls.push('compositor off'),
    enable_unredirect: () => calls.push('compositor on'),
};
unredirect(newer, display, Meta, false);
unredirect(newer, display, Meta, true);
check('newer shells use the compositor', calls.join(', '), 'compositor off, compositor on');
calls.length = 0;
unredirect({}, display, Meta, false);
unredirect({}, display, Meta, true);
check('GNOME 46 uses the display functions', calls.join(', '), 'meta off display, meta on display');

const seatOf = name => ({get_default_seat: () => name});
const stageWithContext = {context: {get_backend: () => seatOf('context seat')}};
const OldClutter = {get_default_backend: () => seatOf('default seat')};
check('newer shells reach the seat through the stage context',
    defaultSeat(stageWithContext, OldClutter), 'context seat');
check('GNOME 46 falls back to the default backend', defaultSeat({}, OldClutter), 'default seat');

const log = [];
class NewNotification {
    constructor(params) {
        Object.assign(this, params);
    }
}
const newSource = {addNotification: n => log.push(`add ${n.title}: ${n.body} transient=${n.isTransient}`)};
const NewTray = {getSystemSource: () => newSource, Notification: NewNotification};
const shown = persistentNotice(NewTray, {add: () => log.push('tray add')}, 'rayshot', 'empty clipboard');
check('GNOME 46+ adds a lasting notice to the system source', log.join(' | '),
    'add rayshot: empty clipboard transient=false');
check('GNOME 46+ returns the notice', shown.source, newSource);

log.length = 0;
class OldNotification {
    constructor(source, title, body) {
        Object.assign(this, {source, title, body, isTransient: false});
    }

    setTransient(value) {
        this.isTransient = value;
        log.push(`transient ${value}`);
    }
}
class OldSource {
    showNotification(n) {
        log.push(`show ${n.title}: ${n.body} transient=${n.isTransient}`);
    }
}
const OldTray = {SystemNotificationSource: OldSource, Notification: OldNotification};
const old = persistentNotice(OldTray, {add: () => log.push('tray add')}, 'rayshot', 'empty clipboard');
check('GNOME 45 adds its own source and shows a lasting notice', log.join(' | '),
    'tray add | transient false | show rayshot: empty clipboard transient=false');
check('GNOME 45 returns the notice', old instanceof OldNotification, true);

print(`${total - failed}/${total} passed`);
if (failed)
    imports.system.exit(1);
