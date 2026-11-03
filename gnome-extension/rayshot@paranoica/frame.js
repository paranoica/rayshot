const MAGIC = [0x52, 0x53, 0x46, 0x31];

function u32(value) {
    if (!Number.isInteger(value) || value < 0 || value > 0xffffffff)
        throw new RangeError(`not a u32: ${value}`);
    return [value & 0xff, (value >>> 8) & 0xff, (value >>> 16) & 0xff, (value >>> 24) & 0xff];
}

export function frameHeader(width, height, stride) {
    return new Uint8Array([...MAGIC, ...u32(width), ...u32(height), ...u32(stride)]);
}
