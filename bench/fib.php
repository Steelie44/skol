<?php
function fib(int $n): int { return $n < 2 ? $n : fib($n - 1) + fib($n - 2); }

$t = hrtime(true);
$r = fib(30);
echo "RESULT $r " . ((hrtime(true) - $t) / 1e9) . "\n";
