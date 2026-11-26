<?php
class Enemy {
    public float $x; public float $y; public float $vx; public float $vy; public int $hp;

    function __construct(int $i) {
        $this->x = (float)($i % 100);
        $this->y = (float)intdiv($i, 100);
        $this->vx = 1.0 + ($i % 7) * 0.1;
        $this->vy = 0.5 + ($i % 5) * 0.1;
        $this->hp = 100;
    }

    function integrate(float $dt): void {
        $this->x += $this->vx * $dt;
        $this->y += $this->vy * $dt;
        if ($this->x > 100.0 || $this->x < 0.0) { $this->vx = -$this->vx; }
        if ($this->y > 100.0 || $this->y < 0.0) { $this->vy = -$this->vy; }
    }

    function damage(int $amount): void {
        $this->hp -= $amount;
        if ($this->hp <= 0) { $this->hp = 100; }
    }
}

function update(array $enemies, float $px, float $py, float $dt): int {
    $hits = 0;
    foreach ($enemies as $e) {
        $e->integrate($dt);
        $dx = $e->x - $px;
        $dy = $e->y - $py;
        if (sqrt($dx * $dx + $dy * $dy) < 10.0) {
            $e->damage(1);
            $hits++;
        }
    }
    return $hits;
}

$enemies = [];
for ($k = 0; $k < 1000; $k++) { $enemies[] = new Enemy($k); }
$total = 0;
$t = hrtime(true);
for ($frame = 0; $frame < 1000; $frame++) {
    $total += update($enemies, 50.0 + (float)($frame % 20), 50.0, 0.5);
}
echo "RESULT $total " . ((hrtime(true) - $t) / 1e9) . "\n";
