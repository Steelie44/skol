extends SceneTree

func _init() -> void:
	var grid: Array[int] = []
	grid.resize(1024)
	grid.fill(0)
	var names := {}
	for k in range(64):
		names["e" + str(k)] = k
	var t := Time.get_ticks_usec()
	var acc := 0
	for rnd in range(2000):
		for k in range(1024):
			grid[k] = grid[k] + k
		acc += names["e" + str(rnd % 64)]
	print("RESULT %d %f" % [acc + grid[1023], (Time.get_ticks_usec() - t) / 1e6])
	quit()
