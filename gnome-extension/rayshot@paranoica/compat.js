export const PROTOCOL_VERSION = 2;

const usable = (path, exists) => typeof path === 'string' && path.startsWith('/') && exists(path);

export function resolveBin(env, setting, inPath, exists) {
    if (usable(env, exists))
        return env;
    if (usable(setting, exists))
        return setting;
    return inPath || null;
}

export function readyVersion(line) {
    if (line === 'ready')
        return 0;
    const m = /^ready (\d+)$/.exec(line);
    return m ? Number(m[1]) : null;
}

export function unredirect(compositor, display, Meta, enabled) {
    if (typeof compositor?.disable_unredirect === 'function') {
        if (enabled)
            compositor.enable_unredirect();
        else
            compositor.disable_unredirect();
    } else if (enabled) {
        Meta.enable_unredirect_for_display(display);
    } else {
        Meta.disable_unredirect_for_display(display);
    }
}

export function defaultSeat(stage, Clutter) {
    const backend = stage.context?.get_backend() ?? Clutter.get_default_backend();
    return backend.get_default_seat();
}

export function persistentNotice(MessageTray, tray, title, body) {
    if (typeof MessageTray.getSystemSource === 'function') {
        const source = MessageTray.getSystemSource();
        const notice = new MessageTray.Notification({source, title, body, isTransient: false});
        source.addNotification(notice);
        return notice;
    }
    const source = new MessageTray.SystemNotificationSource();
    tray.add(source);
    const notice = new MessageTray.Notification(source, title, body);
    notice.setTransient(false);
    source.showNotification(notice);
    return notice;
}

export function monitorManager(global, Meta) {
    return global.backend?.get_monitor_manager?.() ?? Meta.MonitorManager?.get?.() ?? null;
}
