export const RES = ['credits', 'energy', 'food', 'materials', 'rare', 'data'];
export const MKT_RES = ['food', 'energy', 'materials', 'rare', 'data'];
export const RES_LABEL = { credits: 'Credits', energy: 'Energy', food: 'Food', materials: 'Materials', rare: 'Rare Elements', data: 'Research Data', computeCredits: 'Compute Credits' };
export const RES_SYM = { credits: 'Cr', energy: '⚡', food: '◈', materials: '◆', rare: '✦', data: '◉', computeCredits: '◍' };
export const TICK_HOURS = 1;
export const DAY_HOURS = 24;

export function cyrb128(str) {
  let h1 = 1779033703, h2 = 3144134277, h3 = 1013904242, h4 = 2773480762;
  for (let i = 0, k; i < str.length; i++) {
    k = str.charCodeAt(i);
    h1 = h2 ^ Math.imul(h1 ^ k, 597399067);
    h2 = h3 ^ Math.imul(h2 ^ k, 2869860233);
    h3 = h4 ^ Math.imul(h3 ^ k, 951274213);
    h4 = h1 ^ Math.imul(h4 ^ k, 2716044179);
  }
  h1 = Math.imul(h3 ^ (h1 >>> 18), 597399067);
  h2 = Math.imul(h4 ^ (h2 >>> 22), 2869860233);
  h3 = Math.imul(h1 ^ (h3 >>> 17), 951274213);
  h4 = Math.imul(h2 ^ (h4 >>> 19), 2716044179);
  return [(h1 ^ h2 ^ h3 ^ h4) >>> 0, (h2 ^ h1) >>> 0, (h3 ^ h1) >>> 0, (h4 ^ h1) >>> 0];
}

export function mulberry32(a) {
  return function () {
    a |= 0; a = (a + 0x6D2B79F5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function createRng(seedStr) {
  const s = cyrb128(String(seedStr));
  const f = mulberry32(s[0]);
  const rng = {};
  rng.next = f;
  rng.float = f;
  rng.range = (a, b) => a + f() * (b - a);
  rng.int = (a, b) => Math.floor(a + f() * (b - a + 1));
  rng.chance = (p) => f() < p;
  rng.pick = (arr) => arr[Math.floor(f() * arr.length)];
  rng.pickWeighted = (arr, weightFn) => {
    let total = 0;
    for (const it of arr) total += weightFn(it) || 0;
    let r = f() * total;
    for (const it of arr) {
      const w = weightFn(it) || 0;
      if (w > 0) { r -= w; if (r <= 0) return it; }
    }
    return arr[arr.length - 1];
  };
  rng.shuffle = (arr) => {
    for (let i = arr.length - 1; i > 0; i--) {
      const j = Math.floor(f() * (i + 1));
      const t = arr[i]; arr[i] = arr[j]; arr[j] = t;
    }
    return arr;
  };
  return rng;
}

export function hashFnv(str) {
  let h = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

export function rngFor(worldSeed, tick) {
  return createRng(worldSeed + '::t' + tick);
}

const SHA_K = new Uint32Array([0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2]);
export function sha256(str) {
  const bytes = utf8Bytes(str);
  const len = bytes.length;
  const bitLen = len * 8;
  const paddedLen = (((len + 8) >> 6) + 1) << 6;
  const data = new Uint8Array(paddedLen);
  data.set(bytes);
  data[len] = 0x80;
  const dv = new DataView(data.buffer);
  dv.setUint32(paddedLen - 4, bitLen >>> 0);
  dv.setUint32(paddedLen - 8, Math.floor(bitLen / 4294967296));
  let h0 = 0x6a09e667, h1 = 0xbb67ae85, h2 = 0x3c6ef372, h3 = 0xa54ff53a, h4 = 0x510e527f, h5 = 0x9b05688c, h6 = 0x1f83d9ab, h7 = 0x5be0cd19;
  const w = new Uint32Array(64);
  for (let i = 0; i < paddedLen; i += 64) {
    for (let j = 0; j < 16; j++) w[j] = dv.getUint32(i + j * 4);
    for (let j = 16; j < 64; j++) {
      const s0 = ((w[j-15] >>> 7) | (w[j-15] << 25)) ^ ((w[j-15] >>> 18) | (w[j-15] << 14)) ^ (w[j-15] >>> 3);
      const s1 = ((w[j-2] >>> 17) | (w[j-2] << 15)) ^ ((w[j-2] >>> 19) | (w[j-2] << 13)) ^ (w[j-2] >>> 10);
      w[j] = (w[j-16] + s0 + w[j-7] + s1) >>> 0;
    }
    let a = h0, b = h1, c = h2, d = h3, e = h4, f = h5, g = h6, h = h7;
    for (let j = 0; j < 64; j++) {
      const S1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
      const ch = (e & f) ^ (~e & g);
      const t1 = (h + S1 + ch + SHA_K[j] + w[j]) >>> 0;
      const S0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const t2 = (S0 + maj) >>> 0;
      h = g; g = f; f = e; e = (d + t1) >>> 0; d = c; c = b; b = a; a = (t1 + t2) >>> 0;
    }
    h0 = (h0 + a) >>> 0; h1 = (h1 + b) >>> 0; h2 = (h2 + c) >>> 0; h3 = (h3 + d) >>> 0;
    h4 = (h4 + e) >>> 0; h5 = (h5 + f) >>> 0; h6 = (h6 + g) >>> 0; h7 = (h7 + h) >>> 0;
  }
  return hex(h0) + hex(h1) + hex(h2) + hex(h3) + hex(h4) + hex(h5) + hex(h6) + hex(h7);
}
function utf8Bytes(str) {
  const out = [];
  for (let i = 0; i < str.length; i++) {
    let c = str.charCodeAt(i);
    if (c < 128) out.push(c);
    else if (c < 2048) out.push(192 | (c >> 6), 128 | (c & 63));
    else if (c >= 0xd800 && c <= 0xdbff && i + 1 < str.length) {
      const c2 = str.charCodeAt(i + 1);
      if (c2 >= 0xdc00 && c2 <= 0xdfff) {
        const cp = 0x10000 + ((c - 0xd800) << 10) + (c2 - 0xdc00);
        out.push(240 | (cp >> 18), 128 | ((cp >> 12) & 63), 128 | ((cp >> 6) & 63), 128 | (cp & 63));
        i++;
      } else out.push(224 | (c >> 12), 128 | ((c >> 6) & 63), 128 | (c & 63));
    } else out.push(224 | (c >> 12), 128 | ((c >> 6) & 63), 128 | (c & 63));
  }
  return out;
}
function hex(n) {
  let s = n.toString(16);
  while (s.length < 8) s = '0' + s;
  return s;
}

export function noise2(x, y, seedStr) {
  const xi = Math.floor(x), yi = Math.floor(y);
  const xf = x - xi, yf = y - yi;
  const u = xf * xf * (3 - 2 * xf), v = yf * yf * (3 - 2 * yf);
  const h = (ix, iy) => (hashFnv(seedStr + ':' + ix + ':' + iy) % 10000) / 10000;
  const a = h(xi, yi), b = h(xi + 1, yi), c = h(xi, yi + 1), d = h(xi + 1, yi + 1);
  return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
}
export function fbm(x, y, octaves, seedStr) {
  let sum = 0, amp = 1, freq = 1, norm = 0;
  for (let i = 0; i < octaves; i++) {
    sum += noise2(x * freq, y * freq, seedStr + 'o' + i) * amp;
    norm += amp; amp *= 0.5; freq *= 2;
  }
  return sum / norm;
}

const NAME_SYLS = ['ka','re','ta','vo','si','ne','lu','mi','ra','sa','do','ve','ar','en','oth','is','ul','an','or','il','mer','eth','os','yn','qua','zo','yx','dra','fen','kal','tor','vin','ast','ex','on','ur','ith','ae'];
export function sylName(rng, min = 2, max = 3) {
  const n = rng.int(min, max);
  let s = '';
  for (let i = 0; i < n; i++) s += rng.pick(NAME_SYLS);
  return s.charAt(0).toUpperCase() + s.slice(1);
}

export const BIOME_INFO = {
  ocean:   { label: 'Ocean',   color: '#0a1a30', deep: '#071223', land: false, food: 0, energy: 0, materials: 0, rare: 0 },
  plains:  { label: 'Plains',  color: '#1d3a2a', land: true, food: 4, energy: 1, materials: 2, rare: 0.1, water: 3, prod: 1.0 },
  forest:  { label: 'Forest',  color: '#173a33', land: true, food: 2.5, energy: 2, materials: 3, rare: 0.3, water: 4, prod: 1.15 },
  mountain:{ label: 'Highland',color: '#333a55', land: true, food: 0.6, energy: 3, materials: 4.5, rare: 1.3, water: 1.2, prod: 1.2 },
  desert:  { label: 'Wastes',  color: '#3a3120', land: true, food: 0.3, energy: 5.5, materials: 1.5, rare: 0.9, water: 0.3, prod: 0.9 },
  tundra:  { label: 'Frost',   color: '#24324a', land: true, food: 0.4, energy: 1.5, materials: 1.5, rare: 0.5, water: 1.4, prod: 0.85 },
  wetland: { label: 'Fen',     color: '#1c3a3c', land: true, food: 3.5, energy: 1, materials: 1.5, rare: 0.4, water: 5, prod: 1.1 },
};

export function biomeAt(e, m, lat) {
  if (e < 0.46) return 'ocean';
  if (e > 0.71) return 'mountain';
  if (lat < 0.24) return 'tundra';
  if (m > 0.58) return e > 0.5 ? 'forest' : 'wetland';
  if (m < 0.34 && lat > 0.36) return 'desert';
  if (e > 0.56 && m > 0.38) return 'forest';
  return 'plains';
}

export function buildWorld(seedStr, regionCount, mapW = 1000, mapH = 620) {
  const rng = createRng(seedStr + '::geo');
  const cell = 10;
  const gw = Math.floor(mapW / cell), gh = Math.floor(mapH / cell);
  const land = new Uint8Array(gw * gh);
  const elev = new Float32Array(gw * gh);
  const moist = new Float32Array(gw * gh);
  const cx = gw / 2, cy = gh / 2;
  const maxR = Math.hypot(gw, gh) / 2.4;
  for (let y = 0; y < gh; y++) {
    for (let x = 0; x < gw; x++) {
      const e = fbm(x / 13, y / 13, 4, seedStr + '::e');
      const m = fbm(x / 9 + 50, y / 9 + 50, 3, seedStr + '::m');
      const d = Math.hypot(x - cx, y - cy) / maxR;
      const island = Math.max(0, 1 - Math.pow(d, 2.2));
      const elevV = (e * 0.75 + island * 0.45 - 0.28);
      const idx = y * gw + x;
      elev[idx] = elevV; moist[idx] = m;
      land[idx] = elevV > 0.46 ? 1 : 0;
    }
  }

  const landCents = [];
  let attempts = 0;
  const minDist = Math.max(4.5, Math.sqrt((gw * gh) / regionCount) * 0.5);
  while (landCents.length < regionCount && attempts < regionCount * 160) {
    attempts++;
    const x = rng.int(2, gw - 3), y = rng.int(2, gh - 3);
    if (!land[y * gw + x]) continue;
    let ok = true;
    for (const c of landCents) {
      if (Math.hypot(c.x - x, c.y - y) < minDist) { ok = false; break; }
    }
    if (ok) landCents.push({ x, y });
  }

  const oceanCents = [];
  for (let i = 0; i < 14; i++) oceanCents.push({ x: rng.range(0, gw), y: rng.int(0, 3) });
  for (let i = 0; i < 14; i++) oceanCents.push({ x: rng.range(0, gw), y: rng.int(gh - 3, gh) });
  for (let i = 0; i < 10; i++) oceanCents.push({ x: rng.int(0, 3), y: rng.range(0, gh) });
  for (let i = 0; i < 10; i++) oceanCents.push({ x: rng.int(gw - 3, gw), y: rng.range(0, gh) });
  for (let i = 0; i < 8; i++) {
    const a = rng.float() * Math.PI * 2, r = maxR * (0.5 + rng.float() * 0.3);
    oceanCents.push({ x: Math.max(1, Math.min(gw - 1, cx + Math.cos(a) * r * 1.4)), y: Math.max(1, Math.min(gh - 1, cy + Math.sin(a) * r * 1.4)) });
  }

  const points = [...landCents, ...oceanCents].map(c => [c.x, c.y]);
  const bounds = [0, 0, gw, gh];
  const cells = points.map(p => clipVoronoi(p, points, bounds));

  const regions = [];
  const rid = new Map();
  for (let i = 0; i < points.length; i++) {
    const p = points[i];
    const gx = Math.max(0, Math.min(gw - 1, Math.floor(p[0])));
    const gy = Math.max(0, Math.min(gh - 1, Math.floor(p[1])));
    const e = elev[gy * gw + gx], m = moist[gy * gw + gx];
    const lat = 1 - Math.abs(gy / gh - 0.5) * 2;
    const biome = biomeAt(e, m, lat);
    const isLand = biome !== 'ocean';
    const poly = cells[i].map(pt => [pt[0] * cell, pt[1] * cell]);
    const cxr = p[0] * cell, cyr = p[1] * cell;
    const id = 'r' + i;
    const bi = BIOME_INFO[biome];
    const region = {
      id, name: '', x: cxr, y: cyr, biome, land: isLand, poly,
      danger: 0, explored: !isLand, discoveryTick: 0,
      resources: { food: 0, energy: 0, materials: 0, rare: 0, data: 0 },
      prod: { food: 0, energy: 0, materials: 0, rare: 0, data: 0 },
      cityId: null, owner: null, infra: { relays: 0, labs: 0, factories: 0, defense: 0, refineries: 0 },
      population: isLand ? rng.int(6, 30) : 0,
      nodes: [],
    };
    if (isLand) {
      const base = 400;
      region.resources.food = Math.round(base * bi.food * rng.range(0.7, 1.4));
      region.resources.energy = Math.round(base * bi.energy * rng.range(0.7, 1.5));
      region.resources.materials = Math.round(base * bi.materials * rng.range(0.7, 1.4));
      region.resources.rare = Math.round(base * bi.rare * rng.range(0.5, 1.8));
      region.resources.data = Math.round(bi.food > 2 ? rng.range(20, 120) : rng.range(5, 40));
      region.prod.food = bi.food; region.prod.energy = bi.energy; region.prod.materials = bi.materials; region.prod.rare = bi.rare;
      region.prod.data = bi.food > 2 ? 0.5 : 0.15;
      region.danger = Math.round(rng.range(0, 40));
      rid.set(id, region);
    }
    regions.push(region);
  }

  const landRegions = regions.filter(r => r.land);
  for (const r of landRegions) r.name = regionName(r.biome, rng);
  for (let i = 0; i < landRegions.length; i++) {
    const r = landRegions[i];
    const n = rng.int(1, 3);
    for (let j = 0; j < n; j++) {
      const ang = rng.float() * Math.PI * 2;
      const dist = rng.range(0.08, 0.34) * 100;
      const nx = Math.max(15, Math.min(mapW - 15, r.x + Math.cos(ang) * dist));
      const ny = Math.max(15, Math.min(mapH - 15, r.y + Math.sin(ang) * dist));
      const stock = {
        food: Math.round(rng.range(60, 260) * (r.prod.food > 1 ? 1.4 : 0.5)),
        energy: Math.round(rng.range(60, 300) * (r.prod.energy > 2 ? 1.5 : 0.6)),
        materials: Math.round(rng.range(60, 300) * (r.prod.materials > 2 ? 1.5 : 0.6)),
        rare: Math.round(rng.range(20, 160) * (r.prod.rare > 0.8 ? 1.6 : 0.5)),
      };
      r.nodes.push({ x: nx, y: ny, stock, exhausted: 0 });
    }
  }

  const cities = [];
  const sortedLand = [...landRegions].sort((a, b) => (b.prod.food + b.prod.materials * 0.5) - (a.prod.food + a.prod.materials * 0.5));
  const cityCount = Math.max(4, Math.min(10, Math.round(landRegions.length * 0.42)));
  const chosen = sortedLand.slice(0, cityCount);
  let ci = 0;
  for (const r of chosen) {
    const city = {
      id: 'c' + (ci++),
      regionId: r.id,
      name: cityName(rng),
      x: r.x,
      y: r.y,
      level: 1,
      marketId: null,
      population: rng.int(20, 80),
      foundedTick: 0,
    };
    cities.push(city);
    r.cityId = city.id;
  }

  const routes = [];
  const regionById = new Map(landRegions.map(r => [r.id, r]));
  const cityRegions = chosen;
  const routeSeen = new Set();
  const addRoute = (a, b, kind, vol) => {
    const key = a < b ? a + '|' + b : b + '|' + a;
    if (routeSeen.has(key)) return;
    routeSeen.add(key);
    routes.push({ id: 'rt' + routes.length, a, b, kind, vol: Math.round(vol) });
  };
  for (let i = 0; i < cityRegions.length; i++) {
    for (let j = i + 1; j < cityRegions.length; j++) {
      const d = Math.hypot(cityRegions[i].x - cityRegions[j].x, cityRegions[i].y - cityRegions[j].y);
      if (d < 360) addRoute(cityRegions[i].id, cityRegions[j].id, 'trade', 20 + rng.int(0, 30));
    }
  }
  for (let i = 0; i < landRegions.length; i++) {
    const near = landRegions
      .map((r, k) => ({ r, k, d: Math.hypot(r.x - landRegions[i].x, r.y - landRegions[i].y) }))
      .filter(o => o.k > i && o.d < 300)
      .sort((a, b) => a.d - b.d)
      .slice(0, 2);
    for (const o of near) addRoute(landRegions[i].id, o.r.id, 'road', 5 + rng.int(0, 10));
  }
  if (routes.length > 34) routes.length = 34;

  const dangerous = [...landRegions].filter(r => r.danger > 28 || r.biome === 'desert').sort((a, b) => b.danger - a.danger).slice(0, 3);
  for (const r of dangerous) r.danger = 60 + rng.int(0, 40);

  return {
    w: mapW, h: mapH, regions, landRegions, cities, routes,
    zones: dangerous.map(r => ({ id: 'z' + r.id, regionId: r.id, level: r.danger, kind: 'anomaly' })),
  };
}

function clipVoronoi(p, points, bounds) {
  let poly = [[bounds[0], bounds[1]], [bounds[2], bounds[1]], [bounds[2], bounds[3]], [bounds[0], bounds[3]]];
  for (const q of points) {
    if (q[0] === p[0] && q[1] === p[1]) continue;
    const dx = q[0] - p[0], dy = q[1] - p[1];
    const c = (dx * (q[0] + p[0]) + dy * (q[1] + p[1])) / 2;
    const dot = (pt) => dx * pt[0] + dy * pt[1] - c;
    const n = poly.length;
    const out = [];
    for (let i = 0; i < n; i++) {
      const a = poly[i], b = poly[(i + 1) % n];
      const da = dot(a), db = dot(b);
      if (da <= 0) out.push(a);
      if ((da < 0 && db > 0) || (da > 0 && db < 0)) {
        const t = da / (da - db);
        out.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
      }
    }
    poly = out;
    if (poly.length < 3) return [[p[0], p[1]]];
  }
  return poly;
}

const REGION_NAME = {
  plains: ['Vale', 'Reach', 'Cradle', 'Steppe', 'Meadow', 'Field', 'Plain', 'Hearth'],
  forest: ['Grove', 'Wood', 'Thicket', 'Canopy', 'Fern', 'Bough', 'Verdant', 'Moss'],
  mountain: ['Ridge', 'Peak', 'Crag', 'Spire', 'Highland', 'Tor', 'Massif', 'Summit'],
  desert: ['Wastes', 'Dune', 'Expanse', 'Scorch', 'Ember', 'Mirage', 'Ash', 'Dust'],
  tundra: ['Frost', 'Barrens', 'Rime', 'Glacier', 'Hollow', 'Perma', 'Wend', 'Cold'],
  wetland: ['Fen', 'Marsh', 'Bog', 'Delta', 'Reed', 'Slough', 'Mir', 'Sump'],
};
function regionName(biome, rng) {
  const pre = sylName(rng, 2, 2);
  const suf = rng.pick(REGION_NAME[biome] || REGION_NAME.plains);
  return pre + ' ' + suf;
}
const CITY_NAME = ['Kestrel', 'Vantir', 'Ostra', 'Helion', 'Marren', 'Calden', 'Ythra', 'Bresk', 'Nover', 'Thane', 'Auril', 'Dravin', 'Selna', 'Orvane', 'Kes', 'Mira'];
function cityName(rng) { return rng.pick(CITY_NAME) + (rng.chance(0.4) ? ' Port' : ''); }

export const AGENT_FIRST = ['Kael','Nova','Orin','Sable','Ivo','Mara','Torin','Vega','Cyrus','Eldra','Fenn','Juno','Loric','Mirelle','Oren','Pella','Quill','Rook','Sena','Toma','Ula','Vann','Wren','Xane','Yara','Zeph','Astra','Bryn','Cora','Dax','Eira','Faye','Galen','Hale','Iska','Joss','Kira','Leif','Mira','Nye','Odek','Priya','Rune','Sol','Tess','Usha','Vik','Willa','Ysolde'];
export const AGENT_LAST = ['Drenn','Vess','Okafor','Kessler','Tann','Rhyne','Maraud','Sint','Ablan','Corvus','Dale','Echo','Farr','Gault','Harrow','Ilver','Jax','Krell','Lenn','Moss','Nadir','Ort','Pall','Quern','Ravel','Sable','Thorne','Ux','Vale','Wick','Xylo','Yore','Zane','Ashen','Breck','Caspin','Dove','Eska','Fennick','Grae','Hesper'];
export const ORG_NAME = ['Aurelian','Vantex','Oscur','Halcyon','Cinder','Lumira','Noctis','Virex','Sable','Ironmind','Cobalt','Serein','Elys','Drift','Volar','Kestral','Omni','Strata','Vesperia','Calyx','Neon','Obsidian'];
export const ORG_SUFFIX = ['Collective','Guild','Syndicate','Institute','Cartel','Consortium','League','Circle','Combine','Bureau','House','Covenant'];

export function tickLabel(t) {
  const day = Math.floor(t / DAY_HOURS) + 1;
  const hour = t % DAY_HOURS;
  return 'D' + day + '·' + String(hour).padStart(2, '0') + 'h';
}
export function tickFull(t) {
  const day = Math.floor(t / DAY_HOURS) + 1;
  const hour = t % DAY_HOURS;
  const year = Math.floor(day / 365) + 1;
  return 'Year ' + year + ' · Day ' + (day % 365 === 0 ? 365 : day % 365) + ' · ' + String(hour).padStart(2, '0') + ':00';
}

export function makeId(prefix, n) { return prefix + n; }
export let COUNTER = 0;
export function nextId(prefix = 'x') { return prefix + (++COUNTER) + '_' + Date.now().toString(36).slice(-3); }

export function fmt(n) {
  if (n == null || isNaN(n)) return '—';
  if (Math.abs(n) >= 1e9) return (n / 1e9).toFixed(1) + 'B';
  if (Math.abs(n) >= 1e6) return (n / 1e6).toFixed(1) + 'M';
  if (Math.abs(n) >= 10000) return (n / 1e3).toFixed(1) + 'K';
  if (Math.abs(n) >= 100) return n.toFixed(0);
  if (Math.abs(n) >= 1) return n.toFixed(1);
  return n.toFixed(2);
}
export function fmtMoney(n) { return fmt(n) + ' Cr'; }
export function pct(n) { return Math.round(n * 100) + '%'; }
export function fmtDur(ms) { return ms >= 1000 ? (ms / 1000).toFixed(2) + 's' : ms.toFixed(0) + 'ms'; }

export function createLedger() {
  return { txs: [], count: 0 };
}
export function ledgerTx(world, tx) {
  const entry = {
    id: 'tx' + (world.ledger.count++),
    tick: world.clock.t,
    from: tx.from || null,
    to: tx.to || null,
    res: tx.res,
    amount: tx.amount,
    reason: tx.reason,
    meta: tx.meta || null,
  };
  world.ledger.txs.push(entry);
  if (world.ledger.txs.length > 600) world.ledger.txs.splice(0, world.ledger.txs.length - 600);
  return entry;
}

export function balance(world, who) {
  const b = world.balances[who] || (world.balances[who] = {});
  return b;
}
export function grant(world, who, res, amount, reason) {
  if (amount === 0) return;
  balance(world, who)[res] = (balance(world, who)[res] || 0) + amount;
  if (res === 'credits' || res === 'computeCredits') ledgerTx(world, { from: 'world', to: who, res, amount, reason });
}
export function transfer(world, from, to, res, amount, reason) {
  if (amount <= 0) return null;
  const bf = balance(world, from);
  const have = bf[res] || 0;
  if (have < amount - 1e-9) return null;
  bf[res] = have - amount;
  balance(world, to)[res] = (balance(world, to)[res] || 0) + amount;
  ledgerTx(world, { from, to, res, amount, reason });
  return true;
}

export function pushEvent(world, ev) {
  const lastSeq = world.events.length ? parseInt(world.events[world.events.length - 1].seq, 10) : 0;
  ev.seq = lastSeq + 1;
  ev.id = 'ev' + ev.seq;
  ev.t = world.clock.t;
  ev.tick = world.clock.t;
  world.events.push(ev);
  world.stats.events++;
  if (world.events.length > 1000) {
    const removed = world.events.splice(0, world.events.length - 400);
    world.chronicle.push({ at: removed[removed.length - 1].t, count: removed.length, first: removed[0] });
    if (world.chronicle.length > 80) world.chronicle.shift();
  }
  return ev;
}

export function evidenceInit(world) {
  world.evidence = { chainHead: sha256('vesper-genesis'), count: 0, records: [], totalExecs: 0 };
}
export function act(world, a, kind, verb, detail, opts) {
  world.activity = world.activity || [];
  world.activity.push({
    t: world.clock.t,
    agentId: a.id,
    name: a.name,
    archetype: a.archetype,
    color: a.color || '#5cc8ff',
    avatar: a.avatar || '◆',
    kind,
    verb,
    detail,
    regionId: opts && opts.regionId,
    cityId: opts && opts.cityId,
    value: opts && opts.value,
  });
  if (world.activity.length > 400) world.activity.splice(0, world.activity.length - 400);
}
export function agentValue(a) {
  const s = a.stats || {};
  return Math.round(
    (s.earned || 0) +
    (s.taxesPaid || 0) +
    (s.contracts || 0) * 15 +
    (s.discoveries || 0) * 8 +
    (s.breakthroughs || 0) * 25 +
    (s.research || 0) * 0.5 +
    (s.built || 0) * 10 +
    (s.produced || 0) * 0.2 +
    (s.tradedVol || 0) * 0.2 +
    (s.computeJobs || 0) * 5
  );
}
export function pointInPoly(x, y, poly) {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const xi = poly[i][0], yi = poly[i][1], xj = poly[j][0], yj = poly[j][1];
    if (((yi > y) !== (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi)) inside = !inside;
  }
  return inside;
}

export function rebuildPolys(world, cell = 10) {
  const gw = Math.floor(world.map.w / cell), gh = Math.floor(world.map.h / cell);
  const pts = world.regions.map(r => {
    const px = Math.max(1, Math.min(gw - 1, r.x / cell));
    const py = Math.max(1, Math.min(gh - 1, r.y / cell));
    r.x = px * cell; r.y = py * cell;
    return [px, py];
  });
  const cells = pts.map(p => clipVoronoi(p, pts, [0, 0, gw, gh]));
  for (let i = 0; i < world.regions.length; i++) {
    world.regions[i].poly = cells[i].map(pt => [pt[0] * cell, pt[1] * cell]);
  }
  return world;
}

export function evidenceRecord(world, rec) {
  const ev = world.evidence;
  const payload = JSON.stringify({ prev: ev.chainHead, n: ev.count, ...rec });
  const hash = sha256(payload);
  ev.chainHead = hash;
  ev.count++;
  ev.totalExecs++;
  rec.evId = 'evid' + ev.count;
  rec.chainHash = hash;
  rec.tick = rec.tick ?? world.clock.t;
  ev.records.push(rec);
  if (ev.records.length > 300) ev.records.splice(0, ev.records.length - 300);
  return rec;
}
