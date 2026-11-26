<?php
function main(): void {
    $t = hrtime(true);
    $acc = 0;
    $n = 0;
    while ($n < 10000000) {
        $acc += $n * 2 - 1;
        $n++;
    }
    echo "RESULT $acc " . ((hrtime(true) - $t) / 1e9) . "\n";
}
main();
