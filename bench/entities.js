class Enemy {
  constructor(i) {
    this.x = i % 100;
    this.y = Math.floor(i / 100);
    this.vx = 1.0 + (i % 7) * 0.1;
    this.vy = 0.5 + (i % 5) * 0.1;
    this.hp = 100;
  }
  integrate(dt) {
    this.x += this.vx * dt;
    this.y += this.vy * dt;
    if (this.x > 100.0 || this.x < 0.0) this.vx = -this.vx;
    if (this.y > 100.0 || this.y < 0.0) this.vy = -this.vy;
  }
  damage(amount) {
    this.hp -= amount;
    if (this.hp <= 0) this.hp = 100;
  }
}

function update(enemies, px, py, dt) {
  let hits = 0;
  for (const e of enemies) {
    e.integrate(dt);
    const dx = e.x - px;
    const dy = e.y - py;
    if (Math.sqrt(dx * dx + dy * dy) < 10.0) {
      e.damage(1);
      hits++;
    }
  }
  return hits;
}

const enemies = [];
for (let k = 0; k < 1000; k++) enemies.push(new Enemy(k));
let total = 0;
const t = performance.now();
for (let frame = 0; frame < 1000; frame++) {
  total += update(enemies, 50.0 + (frame % 20), 50.0, 0.5);
}
console.log(`RESULT ${total} ${(performance.now() - t) / 1000}`);
