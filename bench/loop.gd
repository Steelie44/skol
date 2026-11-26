extends SceneTree

func _init() -> void:
	var t := Time.get_ticks_usec()
	var acc: int = 0
	var n: int = 0
	while n < 10000000:
		acc += n * 2 - 1
		n += 1
	print("RESULT %d %f" % [acc, (Time.get_ticks_usec() - t) / 1e6])
	quit()
