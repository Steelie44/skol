import time

def fib(n: int) -> int:
    return n if n < 2 else fib(n - 1) + fib(n - 2)

t = time.time()
print(fib(30), time.time() - t)

t = time.time()
acc = 0
n = 0
while n < 10000000:
    acc += n * 2 - 1
    n += 1
print(acc, time.time() - t)