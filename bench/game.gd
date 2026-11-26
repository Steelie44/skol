extends SceneTree

# A mixed "game frame" workload: AI state machines, vector physics, a
# spatial-hash collision pass, sorting, event callbacks, UI string building
# and parsing, string-keyed inventory counts, and BFS pathfinding.

const N := 500
const FRAMES := 1000
const WORLD := 512.0
const CELL := 32
const GRID := 16

var rng_state := 12345
var score := 0
var combo := 0
var best_combo := 0

func rnd(n: int) -> int:
	rng_state = (rng_state * 75 + 74) % 65537
	return rng_state % n

class Entity:
	var id: int
	var x: float
	var y: float
	var vx: float
	var vy: float
	var hp: int
	var state := 0

	func _init(i: int, px: float, py: float, pvx: float, pvy: float, php: int) -> void:
		id = i
		x = px
		y = py
		vx = pvx
		vy = pvy
		hp = php

	func think(px: float, py: float) -> void:
		var dx := px - x
		var dy := py - y
		var d2 := dx * dx + dy * dy
		if d2 >= 10000.0:
			state = 0
		elif hp < 30:
			state = 2
		else:
			state = 1
		if state != 0 and d2 > 0.0:
			var d := sqrt(d2)
			if state == 1:
				vx = dx / d * 2.0
				vy = dy / d * 2.0
			else:
				vx = -dx / d * 3.0
				vy = -dy / d * 3.0

	func integrate() -> void:
		x += vx
		y += vy
		if x < 0.0:
			x = 0.0
			vx = -vx
		elif x > WORLD - 1.0:
			x = WORLD - 1.0
			vx = -vx
		if y < 0.0:
			y = 0.0
			vy = -vy
		elif y > WORLD - 1.0:
			y = WORLD - 1.0
			vy = -vy

func on_kill_score(id: int) -> void:
	score += 10 + id % 3

func on_kill_combo(id: int) -> void:
	combo += 1
	if combo > best_combo:
		best_combo = combo

func on_kill_log(id: int) -> void:
	if id % 50 == 0:
		score += 100

func bfs(walls: Array[int], size: int) -> int:
	var dist: Array[int] = []
	dist.resize(size * size)
	dist.fill(-1)
	var queue: Array[int] = [0]
	dist[0] = 0
	var head := 0
	var goal := size * size - 1
	var dxs: Array[int] = [1, -1, 0, 0]
	var dys: Array[int] = [0, 0, 1, -1]
	while head < queue.size():
		var cur := queue[head]
		head += 1
		if cur == goal:
			return dist[cur]
		var cx := cur % size
		var cy := cur / size
		for k in range(4):
			var nx := cx + dxs[k]
			var ny := cy + dys[k]
			if nx >= 0 and nx < size and ny >= 0 and ny < size:
				var nxt := ny * size + nx
				if walls[nxt] == 0 and dist[nxt] < 0:
					dist[nxt] = dist[cur] + 1
					queue.append(nxt)
	return -1

func _init() -> void:
	var entities: Array[Entity] = []
	for i in range(N):
		var ex := float(rnd(512))
		var ey := float(rnd(512))
		var evx := float(rnd(5)) - 2.0
		var evy := float(rnd(5)) - 2.0
		var ehp := 50 + rnd(50)
		entities.append(Entity.new(i, ex, ey, evx, evy, ehp))
	var walls: Array[int] = []
	for k in range(32 * 32):
		walls.append(1 if rnd(100) < 25 else 0)
	walls[0] = 0
	walls[32 * 32 - 1] = 0
	var listeners: Array[Callable] = [on_kill_score, on_kill_combo, on_kill_log]
	var inventory := {}
	var grid: Array = []
	for k in range(GRID * GRID):
		var cell: Array[int] = []
		grid.append(cell)
	var hits := 0
	var kills := 0
	var checksum := 0

	var t := Time.get_ticks_usec()
	for frame in range(FRAMES):
		var px := float(64 + (frame * 37) % 384)
		var py := float(64 + (frame * 23) % 384)

		for e in entities:
			e.think(px, py)
			e.integrate()

		for cell in grid:
			cell.clear()
		for e in entities:
			var cx := mini(int(e.x) / CELL, GRID - 1)
			var cy := mini(int(e.y) / CELL, GRID - 1)
			grid[cy * GRID + cx].append(e.id)

		combo = 0
		for cell in grid:
			var n: int = cell.size()
			for a in range(n):
				var ea: Entity = entities[cell[a]]
				for b in range(a + 1, n):
					var eb: Entity = entities[cell[b]]
					var dx := ea.x - eb.x
					var dy := ea.y - eb.y
					if dx * dx + dy * dy < 64.0:
						hits += 1
						# push the pair apart
						ea.x += dx * 0.5
						ea.y += dy * 0.5
						eb.x -= dx * 0.5
						eb.y -= dy * 0.5
						for e in [ea, eb]:
							e.hp -= 1
							if e.hp <= 0:
								e.hp = 100
								kills += 1
								for cb in listeners:
									cb.call(e.id)

		if frame % 10 == 0:
			var keyed: Array = []
			for e in entities:
				var dx := e.x - px
				var dy := e.y - py
				keyed.append([int(dx * dx + dy * dy) * 1024 + e.id, e])
			keyed.sort_custom(func(p, q): return p[0] < q[0])
			for k in range(10):
				checksum += keyed[k][1].id

		if frame % 5 == 0:
			var total_hp := 0
			for e in entities:
				total_hp += e.hp
			var line := "frame " + str(frame) + " hp " + str(total_hp) + " kills " + str(kills)
			checksum += line.length()
			var parts := line.split(" ")
			checksum += parts[1].to_int() + parts[3].to_int() % 1000 + parts[5].to_int()

		var item := "item" + str(rnd(20))
		inventory[item] = inventory.get(item, 0) + 1

		if frame % 25 == 0:
			checksum += bfs(walls, 32)

	checksum += hits + kills * 7 + score + best_combo + inventory.get("item7", 0)
	print("RESULT %d %f" % [checksum, (Time.get_ticks_usec() - t) / 1e6])
	quit()
