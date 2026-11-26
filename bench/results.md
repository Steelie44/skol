# Benchmark results

Machine: Windows-11-10.0.26200-SP0, AMD64 Family 26 Model 68 Stepping 0, AuthenticAMD
Best of 3 runs.

| Benchmark | Skol | JS (V8 interp) | Python 3 |
|---|---|---|---|
| fib(30), recursion | 0.040s | 0.034s | 0.060s |
| 10M integer loop | 0.103s | 0.081s | 0.418s |
| 1M entity updates | 0.164s | 0.098s | 0.130s |
| 2M indexed writes + map lookups | 0.058s | 0.022s | 0.033s |
| 200K string builds + counting | 0.045s | 0.011s | 0.022s |
| **Time vs Skol (geomean)** | 1.00× | 0.53× | 1.06× |
