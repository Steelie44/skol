import time

def main():
    t = time.perf_counter()
    acc = 0
    n = 0
    while n < 10000000:
        acc += n * 2 - 1
        n += 1
    print("RESULT", acc, time.perf_counter() - t)

main()
