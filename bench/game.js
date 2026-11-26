// A mixed "game frame" workload: AI state machines, vector physics, a
// spatial-hash collision pass, sorting, event callbacks, UI string building
// and parsing, string-keyed inventory counts, and BFS pathfinding.
const N = 500, FRAMES = 1000, WORLD = 512.0, CELL = 32, GRID = 16;

let seed = 12345;
function rnd(n) {
  seed = (seed * 75 + 74) % 65537;
  return seed % n;
}

class Entity {
  constructor(i) {
    this.id = i;
    this.x = rnd(512);
    this.y = rnd(512);
    this.vx = rnd(5) - 2.0;
    this.vy = rnd(5) - 2.0;
    this.hp = 50 + rnd(50);
    this.state = 0;
  }
  think(px, py) {
    const dx = px - this.x, dy = py - this.y;
    const d2 = dx * dx + dy * dy;
    if (d2 >= 10000.0) this.state = 0;
    else if (this.hp < 30) this.state = 2;
    else this.state = 1;
    if (this.state !== 0 && d2 > 0.0) {
      const d = Math.sqrt(d2);
      if (this.state === 1) { this.vx = dx / d * 2.0; this.vy = dy / d * 2.0; }
      else { this.vx = -dx / d * 3.0; this.vy = -dy / d * 3.0; }
    }
  }
  integrate() {
    this.x += this.vx;
    this.y += this.vy;
    if (this.x < 0.0) { this.x = 0.0; this.vx = -this.vx; }
    else if (this.x > WORLD - 1.0) { this.x = WORLD - 1.0; this.vx = -this.vx; }
    if (this.y < 0.0) { this.y = 0.0; this.vy = -this.vy; }
    else if (this.y > WORLD - 1.0) { this.y = WORLD - 1.0; this.vy = -this.vy; }
  }
}

function bfs(walls, size) {
  const dist = new Array(size * size).fill(-1);
  const queue = [0];
  dist[0] = 0;
  let head = 0;
  const goal = size * size - 1;
  const dxs = [1, -1, 0, 0], dys = [0, 0, 1, -1];
  while (head < queue.length) {
    const cur = queue[head++];
    if (cur === goal) return dist[cur];
    const cx = cur % size, cy = Math.trunc(cur / size);
    for (let k = 0; k < 4; k++) {
      const nx = cx + dxs[k], ny = cy + dys[k];
      if (nx >= 0 && nx < size && ny >= 0 && ny < size) {
        const nxt = ny * size + nx;
        if (walls[nxt] === 0 && dist[nxt] < 0) {
          dist[nxt] = dist[cur] + 1;
          queue.push(nxt);
        }
      }
    }
  }
  return -1;
}

let score = 0, combo = 0, bestCombo = 0;

function main() {
  const entities = [];
  for (let i = 0; i < N; i++) entities.push(new Entity(i));
  const walls = [];
  for (let k = 0; k < 32 * 32; k++) walls.push(rnd(100) < 25 ? 1 : 0);
  walls[0] = 0;
  walls[32 * 32 - 1] = 0;
  const listeners = [
    (id) => { score += 10 + id % 3; },
    (id) => { combo += 1; if (combo > bestCombo) bestCombo = combo; },
    (id) => { if (id % 50 === 0) score += 100; },
  ];
  const inventory = new Map();
  const grid = [];
  for (let k = 0; k < GRID * GRID; k++) grid.push([]);
  let hits = 0, kills = 0, checksum = 0;

  const t = performance.now();
  for (let frame = 0; frame < FRAMES; frame++) {
    const px = 64 + (frame * 37) % 384;
    const py = 64 + (frame * 23) % 384;

    for (const e of entities) { e.think(px, py); e.integrate(); }

    for (const cell of grid) cell.length = 0;
    for (const e of entities) {
      const cx = Math.min(Math.trunc(Math.trunc(e.x) / CELL), GRID - 1);
      const cy = Math.min(Math.trunc(Math.trunc(e.y) / CELL), GRID - 1);
      grid[cy * GRID + cx].push(e.id);
    }

    combo = 0;
    for (const cell of grid) {
      const n = cell.length;
      for (let a = 0; a < n; a++) {
        const ea = entities[cell[a]];
        for (let b = a + 1; b < n; b++) {
          const eb = entities[cell[b]];
          const dx = ea.x - eb.x, dy = ea.y - eb.y;
          if (dx * dx + dy * dy < 64.0) {
            hits += 1;
            // push the pair apart
            ea.x += dx * 0.5; ea.y += dy * 0.5;
            eb.x -= dx * 0.5; eb.y -= dy * 0.5;
            for (const e of [ea, eb]) {
              e.hp -= 1;
              if (e.hp <= 0) {
                e.hp = 100;
                kills += 1;
                for (const cb of listeners) cb(e.id);
              }
            }
          }
        }
      }
    }

    if (frame % 10 === 0) {
      const key = (e) => { const dx = e.x - px, dy = e.y - py; return Math.trunc(dx * dx + dy * dy) * 1024 + e.id; };
      const near = entities.map((e) => [key(e), e]).sort((p, q) => p[0] - q[0]);
      for (let k = 0; k < 10; k++) checksum += near[k][1].id;
    }

    if (frame % 5 === 0) {
      let totalHp = 0;
      for (const e of entities) totalHp += e.hp;
      const line = "frame " + frame + " hp " + totalHp + " kills " + kills;
      checksum += line.length;
      const parts = line.split(" ");
      checksum += parseInt(parts[1], 10) + parseInt(parts[3], 10) % 1000 + parseInt(parts[5], 10);
    }

    const item = "item" + rnd(20);
    inventory.set(item, (inventory.get(item) || 0) + 1);

    if (frame % 25 === 0) checksum += bfs(walls, 32);
  }

  checksum += hits + kills * 7 + score + bestCombo + (inventory.get("item7") || 0);
  console.log("RESULT", checksum, ((performance.now() - t) / 1000).toFixed(6));
}

main();
