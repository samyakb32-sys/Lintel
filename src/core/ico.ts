// A Windows .ico made from pictures the page drew (the icon of METEOR V1's own shortcuts,
// island/tray-mascot.ts). No DOM: the pictures come in as RGBA or as PNG bytes. Same layout as
// scripts/gen-icons.mjs writes for the icon the program carries: a bitmap below 256 px, a PNG at 256.

export interface IcoEntry {
  /** Side in pixels, 1…256. */
  size: number;
  /** A bitmap from `dibEntry`, or a PNG file. */
  data: Uint8Array;
}

/** One square RGBA picture as the bitmap an .ico holds: its header, the rows bottom-up as BGRA, and an empty mask. */
export function dibEntry(rgba: ArrayLike<number>, size: number): Uint8Array {
  const maskRow = Math.ceil(size / 32) * 4;
  const out = new Uint8Array(40 + size * size * 4 + maskRow * size);
  const head = new DataView(out.buffer);
  head.setUint32(0, 40, true);
  head.setInt32(4, size, true);
  head.setInt32(8, size * 2, true); // the picture and its mask
  head.setUint16(12, 1, true);
  head.setUint16(14, 32, true);
  head.setUint32(20, size * size * 4 + maskRow * size, true);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const from = ((size - 1 - y) * size + x) * 4;
      const to = 40 + (y * size + x) * 4;
      out[to] = rgba[from + 2];
      out[to + 1] = rgba[from + 1];
      out[to + 2] = rgba[from];
      out[to + 3] = rgba[from + 3];
    }
  }
  return out;
}

/** The file: a header, one directory entry per picture, then the pictures. */
export function encodeIco(entries: IcoEntry[]): Uint8Array {
  const head = 6 + 16 * entries.length;
  const out = new Uint8Array(head + entries.reduce((n, e) => n + e.data.length, 0));
  const view = new DataView(out.buffer);
  view.setUint16(2, 1, true); // an icon
  view.setUint16(4, entries.length, true);
  let at = head;
  entries.forEach((e, i) => {
    const o = 6 + i * 16;
    out[o] = out[o + 1] = e.size >= 256 ? 0 : e.size; // 0 reads as 256
    view.setUint16(o + 4, 1, true);
    view.setUint16(o + 6, 32, true);
    view.setUint32(o + 8, e.data.length, true);
    view.setUint32(o + 12, at, true);
    out.set(e.data, at);
    at += e.data.length;
  });
  return out;
}
