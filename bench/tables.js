function main() {
  const grid = new Array(1024).fill(0);
  const names = new Map();
  for (let k = 0; k < 64; k++) names.set("e" + k, k);
  const t = performance.now();
  let acc = 0;
  for (let rnd = 0; rnd < 2000; rnd++) {
    for (let k = 0; k < 1024; k++) {
      grid[k] = grid[k] + k;
    }
    acc += names.get("e" + (rnd % 64));
  }
  console.log(`RESULT ${acc + grid[1023]} ${(performance.now() - t) / 1000}`);
}
main();
