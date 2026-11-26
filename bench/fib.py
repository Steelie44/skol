import time

def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)

t = time.perf_counter()
r = fib(30)
print("RESULT", r, time.perf_counter() - t)
