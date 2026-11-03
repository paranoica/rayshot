export function growth(samples, key, warmup = 1) {
    const values = samples.map(s => s[key]).filter(v => typeof v === 'number');
    if (values.length < 2)
        return null;
    return values.at(-1) - values[Math.min(warmup, values.length - 2)];
}

export function restarts(samples) {
    const pids = samples.map(s => s.server_pid).filter(p => p !== null && p !== undefined);
    return pids.filter((p, i) => i > 0 && p !== pids[i - 1]).length;
}
