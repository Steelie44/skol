function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }

const t = performance.now();
const r = fib(30);
console.log(`RESULT ${r} ${(performance.now() - t) / 1000}`);
