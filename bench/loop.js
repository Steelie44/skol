function main() {
  const t = performance.now();
  let acc = 0;
  let n = 0;
  while (n < 10000000) {
    acc += n * 2 - 1;
    n++;
  }
  console.log(`RESULT ${acc} ${(performance.now() - t) / 1000}`);
}
main();
