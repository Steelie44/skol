# A mixed "game frame" workload: AI state machines, vector physics, a
# spatial-hash collision pass, sorting, event callbacks, UI string building
# and parsing, string-keyed inventory counts, and BFS pathfinding.
import math
import time

N = 500
FRAMES = 1000
WORLD = 512.0
CELL = 32
GRID = 16

seed = 12345

def rnd(n):
    global seed
    seed = (seed * 75 + 74) % 65537
    return seed % n

class Entity:
    __slots__ = ("id", "x", "y", "vx", "vy", "hp", "state")

    def __init__(self, i):
        self.id = i
        self.x = float(rnd(512))
        self.y = float(rnd(512))
        self.vx = float(rnd(5)) - 2.0
        self.vy = float(rnd(5)) - 2.0
        self.hp = 50 + rnd(50)
        self.state = 0

    def think(self, px, py):
        dx = px - self.x
        dy = py - self.y
        d2 = dx * dx + dy * dy
        if d2 >= 10000.0:
            self.state = 0
        elif self.hp < 30:
            self.state = 2
        else:
            self.state = 1
        if self.state != 0 and d2 > 0.0:
            d = math.sqrt(d2)
            if self.state == 1:
                self.vx = dx / d * 2.0
                self.vy = dy / d * 2.0
            else:
                self.vx = -dx / d * 3.0
                self.vy = -dy / d * 3.0

    def integrate(self):
        self.x += self.vx
        self.y += self.vy
        if self.x < 0.0:
            self.x = 0.0
            self.vx = -self.vx
        elif self.x > WORLD - 1.0:
            self.x = WORLD - 1.0
            self.vx = -self.vx
        if self.y < 0.0:
            self.y = 0.0
            self.vy = -self.vy
        elif self.y > WORLD - 1.0:
            self.y = WORLD - 1.0
            self.vy = -self.vy

def bfs(walls, size):
    dist = [-1] * (size * size)
    queue = [0]
    dist[0] = 0
    head = 0
    goal = size * size - 1
    while head < len(queue):
        cur = queue[head]
        head += 1
        if cur == goal:
            return dist[cur]
        cx = cur % size
        cy = cur // size
        for nx, ny in ((cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)):
            if 0 <= nx < size and 0 <= ny < size:
                nxt = ny * size + nx
                if walls[nxt] == 0 and dist[nxt] < 0:
                    dist[nxt] = dist[cur] + 1
                    queue.append(nxt)
    return -1

score = 0
combo = 0
best_combo = 0

def on_kill_score(e_id):
    global score
    score += 10 + e_id % 3

def on_kill_combo(e_id):
    global combo, best_combo
    combo += 1
    if combo > best_combo:
        best_combo = combo

def on_kill_log(e_id):
    global score
    if e_id % 50 == 0:
        score += 100

def main():
    global combo
    entities = [Entity(i) for i in range(N)]
    walls = [1 if rnd(100) < 25 else 0 for _ in range(32 * 32)]
    walls[0] = 0
    walls[32 * 32 - 1] = 0
    listeners = [on_kill_score, on_kill_combo, on_kill_log]
    inventory = {}
    grid = [[] for _ in range(GRID * GRID)]
    hits = 0
    kills = 0
    checksum = 0

    t = time.perf_counter()
    for frame in range(FRAMES):
        px = float(64 + (frame * 37) % 384)
        py = float(64 + (frame * 23) % 384)

        for e in entities:
            e.think(px, py)
            e.integrate()

        for cell in grid:
            cell.clear()
        for e in entities:
            cx = min(int(e.x) // CELL, GRID - 1)
            cy = min(int(e.y) // CELL, GRID - 1)
            grid[cy * GRID + cx].append(e.id)

        combo = 0
        for cell in grid:
            n = len(cell)
            for a in range(n):
                ea = entities[cell[a]]
                for b in range(a + 1, n):
                    eb = entities[cell[b]]
                    dx = ea.x - eb.x
                    dy = ea.y - eb.y
                    if dx * dx + dy * dy < 64.0:
                        hits += 1
                        # push the pair apart
                        ea.x += dx * 0.5
                        ea.y += dy * 0.5
                        eb.x -= dx * 0.5
                        eb.y -= dy * 0.5
                        for e in (ea, eb):
                            e.hp -= 1
                            if e.hp <= 0:
                                e.hp = 100
                                kills += 1
                                for f in listeners:
                                    f(e.id)

        if frame % 10 == 0:
            def key(e):
                dx = e.x - px
                dy = e.y - py
                return int(dx * dx + dy * dy) * 1024 + e.id
            near = sorted(entities, key=key)
            for e in near[:10]:
                checksum += e.id

        if frame % 5 == 0:
            total_hp = 0
            for e in entities:
                total_hp += e.hp
            line = "frame " + str(frame) + " hp " + str(total_hp) + " kills " + str(kills)
            checksum += len(line)
            parts = line.split(" ")
            checksum += int(parts[1]) + int(parts[3]) % 1000 + int(parts[5])

        item = "item" + str(rnd(20))
        if item in inventory:
            inventory[item] += 1
        else:
            inventory[item] = 1

        if frame % 25 == 0:
            checksum += bfs(walls, 32)

    checksum += hits + kills * 7 + score + best_combo + inventory.get("item7", 0)
    print("RESULT", checksum, time.perf_counter() - t)

main()
