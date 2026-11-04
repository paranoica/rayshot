// Growth of the floor: the lowest value late in the run against the lowest
// early on, after the warmup. Memory that is freed in bursts saws up and down,
// and only a floor that keeps rising is a leak. The windows sit a third of the
// run apart, so the difference is scaled to the whole run.
export function growth(samples, key, warmup = 1) {
    const values = samples.map(s => s[key]).filter(v => typeof v === 'number');
    if (values.length < 2)
        return null;
    const after = values.slice(Math.min(warmup, values.length - 2));
    const window = Math.max(1, Math.floor(after.length / 3));
    const floorRise = Math.min(...after.slice(-window)) - Math.min(...after.slice(0, window));
    return Math.round(floorRise * (after.length - 1) / (after.length - window));
}
