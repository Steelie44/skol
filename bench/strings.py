import time

def main():
    counts = {}
    total = 0
    t = time.perf_counter()
    for k in range(200000):
        key = "k" + str(k % 1000)
        total += len(key)
        counts[key] = counts.get(key, 0) + 1
    print("RESULT", total + len(counts), time.perf_counter() - t)

main()
