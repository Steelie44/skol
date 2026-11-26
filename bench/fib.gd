extends SceneTree

func fib(n: int) -> int:
	if n < 2:
		return n
	return fib(n - 1) + fib(n - 2)

func _init() -> void:
	var t := Time.get_ticks_usec()
	var r := fib(30)
	print("RESULT %d %f" % [r, (Time.get_ticks_usec() - t) / 1e6])
	quit()
