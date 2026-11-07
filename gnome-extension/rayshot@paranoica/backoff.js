const BASE_MS = 250;
const CAP_MS = 30000;

export function respawnDelay(failures) {
    return Math.min(BASE_MS * 2 ** failures, CAP_MS);
}

export const MAX_FAILURES = 5;

export function shouldRespawn(failures) {
    return failures < MAX_FAILURES;
}
