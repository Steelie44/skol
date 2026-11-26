# Benchmark results

Machine: Windows-11-10.0.26200-SP0, AMD64 Family 26 Model 68 Stepping 0, AuthenticAMD
Best of 3 runs.

| Benchmark | Skol | JS (V8 interp) | Python 3 |
|---|---|---|---|
| fib(30), recursion | 0.031s | 0.034s | 0.059s |
| 10M integer loop | 0.079s | 0.082s | 0.370s |
| 1M entity updates | 0.097s | 0.098s | 0.130s |
| 2M indexed writes + map lookups | 0.023s | 0.022s | 0.034s |
| 200K string builds + counting | 0.025s | 0.012s | 0.022s |
| 1000 game frames: AI, physics, collisions, sorting, events, strings, BFS | 0.228s | 0.242s | 0.304s |
| **Time vs Skol (geomean)** | 1.00× | 0.90× | 1.66× |
