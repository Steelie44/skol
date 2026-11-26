import math
import time

class Enemy:
    __slots__ = ("x", "y", "vx", "vy", "hp")

    def __init__(self, i):
        self.x = float(i % 100)
        self.y = float(i // 100)
        self.vx = 1.0 + (i % 7) * 0.1
        self.vy = 0.5 + (i % 5) * 0.1
        self.hp = 100

    def integrate(self, dt):
        self.x += self.vx * dt
        self.y += self.vy * dt
        if self.x > 100.0 or self.x < 0.0:
            self.vx = -self.vx
        if self.y > 100.0 or self.y < 0.0:
            self.vy = -self.vy

    def damage(self, amount):
        self.hp -= amount
        if self.hp <= 0:
            self.hp = 100

def update(enemies, px, py, dt):
    hits = 0
    for e in enemies:
        e.integrate(dt)
        dx = e.x - px
        dy = e.y - py
        if math.sqrt(dx * dx + dy * dy) < 10.0:
            e.damage(1)
            hits += 1
    return hits

def main():
    enemies = [Enemy(k) for k in range(1000)]
    total = 0
    t = time.perf_counter()
    for frame in range(1000):
        total += update(enemies, 50.0 + float(frame % 20), 50.0, 0.5)
    print("RESULT", total, time.perf_counter() - t)

main()
