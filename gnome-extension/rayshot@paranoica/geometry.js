const INSET = 8;

function contains(r, x, y) {
    return x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
}

export function pickClickPoint(area, popups) {
    if (area.width <= 0 || area.height <= 0)
        return null;

    const left = area.x + INSET;
    const top = area.y + INSET;
    const right = area.x + area.width - 1 - INSET;
    const bottom = area.y + area.height - 1 - INSET;
    const candidates = [
        {x: Math.floor(area.x + area.width / 2), y: Math.floor(area.y + area.height / 2)},
        {x: left, y: top},
        {x: right, y: top},
        {x: left, y: bottom},
        {x: right, y: bottom},
    ];

    return candidates.find(p =>
        contains(area, p.x, p.y) && !popups.some(r => contains(r, p.x, p.y))) ?? null;
}
