import time

def main():
    grid = [0] * 1024
    names = {"e" + str(k): k for k in range(64)}
    t = time.perf_counter()
    acc = 0
    for rnd in range(2000):
        for k in range(1024):
            grid[k] = grid[k] + k
        acc += names["e" + str(rnd % 64)]
    print("RESULT", acc + grid[1023], time.perf_counter() - t)

main()
