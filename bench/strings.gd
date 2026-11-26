extends SceneTree

func _init() -> void:
	var counts := {}
	var total := 0
	var t := Time.get_ticks_usec()
	for k in range(200000):
		var key := "k" + str(k % 1000)
		total += key.length()
		counts[key] = counts.get(key, 0) + 1
	print("RESULT %d %f" % [total + counts.size(), (Time.get_ticks_usec() - t) / 1e6])
	quit()
