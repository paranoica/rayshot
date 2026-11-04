const BASE_MS = 250;
const CAP_MS = 30000;

export function respawnDelay(failures) {
    const n = Number.isFinite(failures) ? Math.max(0, Math.floor(failures)) : 0;
    return Math.min(BASE_MS * 2 ** Math.min(n, 30), CAP_MS);
}

export const MAX_FAILURES = 5;

export function shouldRespawn(failures) {
    return !(failures >= MAX_FAILURES);
}
