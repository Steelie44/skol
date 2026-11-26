<?php
function main(): void {
    $counts = [];
    $total = 0;
    $t = hrtime(true);
    for ($k = 0; $k < 200000; $k++) {
        $key = "k" . ($k % 1000);
        $total += strlen($key);
        $counts[$key] = ($counts[$key] ?? 0) + 1;
    }
    echo "RESULT " . ($total + count($counts)) . " " . ((hrtime(true) - $t) / 1e9) . "\n";
}
main();
