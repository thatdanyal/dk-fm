// Generates build/icon.png (1024x1024 pixel-art logo) with no dependencies.
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const N = 32; // logical grid
const S = 32; // px per cell -> 1024
const grid = Array.from({ length: N }, () => Array(N).fill(null));

const C = { bg: [22, 8, 8], border: [255, 59, 47], red: [255, 59, 47], amber: [255, 174, 59], cream: [255, 217, 207], dim: [70, 22, 18] };

// Rounded-ish tile with stepped corners.
for (let y = 1; y < N - 1; y++) for (let x = 1; x < N - 1; x++) {
  const cx = Math.min(x - 1, N - 2 - x);
  const cy = Math.min(y - 1, N - 2 - y);
  if (cx + cy < 3) continue;
  const edge = cx === 0 || cy === 0 || cx + cy === 3;
  grid[y][x] = edge ? C.border : C.bg;
}
// scanline texture
for (let y = 3; y < N - 3; y += 2) for (let x = 3; x < N - 3; x++) if (grid[y][x] === C.bg) grid[y][x] = [18, 6, 6];

const G = {
  D: ['11110', '10001', '10001', '10001', '10001', '10001', '11110'],
  K: ['10001', '10010', '10100', '11000', '10100', '10010', '10001'],
  F: ['11111', '10000', '10000', '11110', '10000', '10000', '10000'],
  M: ['10001', '11011', '10101', '10101', '10001', '10001', '10001'],
};
const draw = (ch, ox, oy, scale, col, shadow) => {
  G[ch].forEach((row, r) => [...row].forEach((b, c) => {
    if (b !== '1') return;
    for (let dy = 0; dy < scale; dy++) for (let dx = 0; dx < scale; dx++) {
      if (shadow) grid[oy + r * scale + dy + 1][ox + c * scale + dx + 1] ||= shadow;
      if (shadow && grid[oy + r * scale + dy + 1][ox + c * scale + dx + 1] !== col) grid[oy + r * scale + dy + 1][ox + c * scale + dx + 1] = shadow;
    }
  }));
  G[ch].forEach((row, r) => [...row].forEach((b, c) => {
    if (b !== '1') return;
    for (let dy = 0; dy < scale; dy++) for (let dx = 0; dx < scale; dx++) grid[oy + r * scale + dy][ox + c * scale + dx] = col;
  }));
};
draw('D', 5, 5, 2, C.red, C.dim);
draw('K', 17, 5, 2, C.red, C.dim);
// ".FM"
grid[26][9] = C.amber; grid[26][10] = C.amber; grid[25][9] = C.amber; grid[25][10] = C.amber;
draw('F', 12, 20, 1, C.cream);
draw('M', 18, 20, 1, C.cream);
// little EQ bars on the right of FM
[[24, 4], [26, 6], [28, 3]].forEach(([x, hgt]) => { for (let k = 0; k < hgt; k++) grid[26 - k][x] = k > 3 ? C.cream : C.amber; });

// Rasterize
const W = N * S;
const raw = Buffer.alloc((W * 4 + 1) * W);
for (let y = 0; y < W; y++) {
  raw[y * (W * 4 + 1)] = 0;
  for (let x = 0; x < W; x++) {
    const c = grid[Math.floor(y / S)][Math.floor(x / S)];
    const o = y * (W * 4 + 1) + 1 + x * 4;
    if (c) { raw[o] = c[0]; raw[o + 1] = c[1]; raw[o + 2] = c[2]; raw[o + 3] = 255; }
  }
}
const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
const crc = (buf) => { let c = 0xffffffff; for (const b of buf) c = crcTable[(c ^ b) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
const chunk = (type, data) => {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const c = Buffer.alloc(4); c.writeUInt32BE(crc(td));
  return Buffer.concat([len, td, c]);
};
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(W, 0); ihdr.writeUInt32BE(W, 4); ihdr[8] = 8; ihdr[9] = 6;
const png = Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
const out = path.join(__dirname, '..', 'build', 'icon.png');
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, png);
console.log('wrote', out, png.length, 'bytes');
