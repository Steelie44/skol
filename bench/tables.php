<?php
function main(): void {
    $grid = array_fill(0, 1024, 0);
    $names = [];
    for ($k = 0; $k < 64; $k++) { $names["e" . $k] = $k; }
    $t = hrtime(true);
    $acc = 0;
    for ($rnd = 0; $rnd < 2000; $rnd++) {
        for ($k = 0; $k < 1024; $k++) {
            $grid[$k] = $grid[$k] + $k;
        }
        $acc += $names["e" . ($rnd % 64)];
    }
    echo "RESULT " . ($acc + $grid[1023]) . " " . ((hrtime(true) - $t) / 1e9) . "\n";
}
main();
