export const PROTOCOL_VERSION = 1;

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
