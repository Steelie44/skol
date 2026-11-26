function main() {
  const counts = new Map();
  let total = 0;
  const t = performance.now();
  for (let k = 0; k < 200000; k++) {
    const key = "k" + (k % 1000);
    total += key.length;
    counts.set(key, (counts.get(key) || 0) + 1);
  }
  console.log(`RESULT ${total + counts.size} ${(performance.now() - t) / 1000}`);
}
main();
