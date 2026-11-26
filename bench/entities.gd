extends SceneTree

class Enemy:
	var x: float
	var y: float
	var vx: float
	var vy: float
	var hp: int

	func _init(i: int) -> void:
		x = float(i % 100)
		y = float(i / 100)
		vx = 1.0 + (i % 7) * 0.1
		vy = 0.5 + (i % 5) * 0.1
		hp = 100

	func integrate(dt: float) -> void:
		x += vx * dt
		y += vy * dt
		if x > 100.0 or x < 0.0:
			vx = -vx
		if y > 100.0 or y < 0.0:
			vy = -vy

	func damage(amount: int) -> void:
		hp -= amount
		if hp <= 0:
			hp = 100

func update(enemies: Array[Enemy], px: float, py: float, dt: float) -> int:
	var hits := 0
	for e in enemies:
		e.integrate(dt)
		var dx := e.x - px
		var dy := e.y - py
		if sqrt(dx * dx + dy * dy) < 10.0:
			e.damage(1)
			hits += 1
	return hits

func _init() -> void:
	var enemies: Array[Enemy] = []
	for k in range(1000):
		enemies.append(Enemy.new(k))
	var total := 0
	var t := Time.get_ticks_usec()
	for frame in range(1000):
		total += update(enemies, 50.0 + float(frame % 20), 50.0, 0.5)
	print("RESULT %d %f" % [total, (Time.get_ticks_usec() - t) / 1e6])
	quit()
